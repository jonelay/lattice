use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use lattice_core::fuse::{assemble, load_fuse_profile, load_manifest, parse_trace};
use lattice_core::output::output_result;
use serde_json::{Value, json};

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
) -> lattice_core::types::FuseReport {
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
const PROFILE: &str = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: hint}\n";

// Requirement: Read a fuse manifest
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
    for invalid in [
        original.replace("name: a", "name: z"),
        original.replace("name: z", "name: z:q"),
        original.replace("profile: z.yaml", "profile: /absolute"),
        original.replace("version: '2.0.0'", "version: 2"),
        original.replace("fuse_profile:", "program_profile:"),
        "[]".into(),
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(load_manifest(&path).is_err());
    }
}

// Requirement: Source-qualified merge with colon separator
// Requirement: Pathway preservation
// Requirement: Tri-format output
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
    let json: Value = serde_json::from_str(&output_result(&report, "json").unwrap()).unwrap();
    assert_eq!(json["header"]["name"], "sample");
    assert_eq!(json["nodes"][0]["id"], "z:Z");
    assert_eq!(json["nodes"][1]["kind"], "a:item");
    assert_eq!(json["edges"][0]["src"], "z:Z");
    assert_eq!(json["edges"][0]["tgt"], "a:A");
    assert_eq!(json["edges"][0]["attrs"]["weight"], 2);
    assert_eq!(json["edges"][0]["provenance"]["source"], "z");
    assert_eq!(json["pathways"][0]["name"], "z:phase");
    assert_eq!(json["pathways"][1]["name"], "a:phase");
    assert_eq!(json["pathways"][0]["order"], json!(["first", "last"]));
    for format in ["plain", "rich"] {
        let text = output_result(&report, format).unwrap();
        assert!(text.contains("z:Z"));
        assert!(text.contains("a:A"));
        assert!(text.contains("z:phase"));
    }
}

// Requirement: Standard validators on the composed graph
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

// Requirement: Cross-source edge resolution via allowed pairings
// Requirement: Standard validators on the composed graph
#[test]
fn duplicates_ambiguity_dangling_coverage_and_strict() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item], [z:item, z:item]]\nvalidations:\n  - CROSS_SOURCE_DUPLICATE_ID: {severity: warning}\n  - AMBIGUOUS_CROSS_REF: {severity: hint}\n  - VACANCY: {severity: warning}\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: hint}\n";
    let z = trace(json!([entry("X", json!([edge("X"), edge("absent")]))]));
    let a = trace(json!([entry("X", json!([]))]));
    let normal = report(&dir, z.clone(), a.clone(), profile, false);
    assert_eq!(normal.exit_code(), 0);
    let value: Value = serde_json::from_str(&output_result(&normal, "json").unwrap()).unwrap();
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
            .all(|f| f.issue.severity == lattice_core::types::Severity::Hint)
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

// Requirement: Standard validators on the composed graph
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

// Requirement: Standard validators on the composed graph
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

// Requirement: Standard validators on the composed graph
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

// Requirement: Standard validators on the composed graph
#[test]
fn coverage_repeated_entries_share_per_code_severity() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: error}\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: hint}\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let result = report(&dir, z, a, profile, false);
    assert_eq!(result.exit_code(), 0);
    let coverage_findings: Vec<_> = result
        .findings
        .iter()
        .filter(|f| f.issue.code == "COVERAGE")
        .collect();
    assert!(!coverage_findings.is_empty(), "expected COVERAGE findings");
    assert!(
        coverage_findings
            .iter()
            .all(|f| f.issue.severity == lattice_core::types::Severity::Hint),
        "per-code severity should be hint (last-declared)"
    );
}

// Requirement: Standard validators on the composed graph
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

// Requirement: Standard validators on the composed graph
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

// Requirement: Standard validators on the composed graph
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

// Requirement: Read a fuse profile
// Requirement: Run each source through lattice trace
#[test]
fn rejects_bad_profiles_and_traces() {
    let dir = Scratch::new();
    for invalid in [
        "node_kinds: {}\nedge_kinds: {}",
        "edge_kinds: []",
        "edge_kinds: {links: {allowed: [[z/item, a/item]]}}",
        "edge_kinds: {}\nvalidations: [{COVERAGE: {target_kind: 'a:item'}}]",
        "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n      where: {title: {badop: 1}}\n",
        "edge_kinds: {}\nvalidations: [{VACANCY: {severity: fatal}}]",
        "edge_kinds: {}\nmisspelled: true",
    ] {
        assert!(
            load_fuse_profile(&dir.write("profile.yaml", invalid)).is_err(),
            "{invalid}"
        );
    }
    for invalid in [
        "not json".into(),
        "{}".into(),
        trace(json!([{"id": "A"}])).to_string(),
        trace(json!([entry("A", json!([{"tgt": "B"}]))])).to_string(),
    ] {
        assert!(parse_trace(&invalid, "z").is_err());
    }
}

fn cli(path: &Path, format: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(["fuse", "--manifest"])
        .arg(path)
        .args(["--format", format])
        .output()
        .unwrap()
}
// Requirement: Fuse exit codes
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
// Requirement: Source-qualified merge with colon separator
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

// Requirement: Standard validators on the composed graph
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

// Requirement: Source-qualified merge with colon separator
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

// Requirement: Cross-source edge resolution via allowed pairings
#[test]
fn target_must_match_an_allowed_kind_even_if_the_raw_id_exists() {
    let dir = Scratch::new();
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
    use std::os::unix::fs::PermissionsExt;
    let path = dir.write(name, text);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

// Requirement: Run each source through lattice trace
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
    let report = lattice_core::fuse::fuse(&manifest, &binary, true).unwrap();
    assert_eq!(report.exit_code(), 2);
    assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
    assert_eq!(report.findings[0].issue.code, "LOCAL");
    assert_eq!(report.findings[0].source.as_deref(), Some("z"));
    assert_eq!(report.findings[1].issue.code, "SOURCE_FAILURE");
    assert_eq!(report.findings[1].source.as_deref(), Some("a"));
}

// Requirement: Run each source through lattice trace
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
        let report = lattice_core::fuse::fuse(&manifest, &binary, false).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
        assert_eq!(report.findings.len(), 2);
        for (finding, source) in report.findings.iter().zip(["z", "a"]) {
            assert_eq!(finding.issue.code, "SOURCE_FAILURE");
            assert_eq!(finding.issue.severity, lattice_core::types::Severity::Error);
            assert_eq!(finding.source.as_deref(), Some(source));
            assert_eq!(
                finding.issue.message,
                format!("source '{source}' emitted invalid trace JSON: {reason}")
            );
        }
    }
}

// Requirement: Run each source through lattice trace
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
    let binary = Path::new(env!("CARGO_BIN_EXE_lattice"));
    for (healthy, failed) in [("z", "a"), ("a", "z")] {
        executable(
            &dir,
            healthy,
            &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n"),
        );
        executable(&dir, failed, "#!/bin/sh\necho broken >&2\nexit 2\n");
        let report = lattice_core::fuse::fuse(&manifest, binary, false).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
        assert_eq!(report.findings.len(), 3);
        for (finding, message) in report.findings[..2].iter().zip(["first", "second"]) {
            assert_eq!(finding.issue.code, "PARSE_ERROR");
            assert_eq!(finding.issue.message, message);
            assert_eq!(finding.issue.severity, lattice_core::types::Severity::Error);
            assert_eq!(finding.source.as_deref(), Some(healthy));
        }
        assert_eq!(report.findings[2].issue.code, "SOURCE_FAILURE");
        assert_eq!(report.findings[2].source.as_deref(), Some(failed));
    }
}

// Requirement: Run each source through lattice trace
// Requirement: Standard validators on the composed graph
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
    let binary = Path::new(env!("CARGO_BIN_EXE_lattice"));
    for (strict, severity) in [
        (false, lattice_core::types::Severity::Warning),
        (true, lattice_core::types::Severity::Error),
    ] {
        let report = lattice_core::fuse::fuse(&manifest, binary, strict).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.findings[0].issue.code, "PARSE_ERROR");
        assert_eq!(report.findings[0].issue.severity, severity);
        assert_eq!(report.findings[0].source.as_deref(), Some("z"));
        assert_eq!(report.findings[1].issue.code, "SOURCE_FAILURE");
        assert_eq!(
            report.findings[1].issue.severity,
            lattice_core::types::Severity::Error
        );
        assert_eq!(report.findings[1].source.as_deref(), Some("a"));
    }
}

// Requirement: Run each source through lattice trace
// Requirement: Fuse exit codes
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
        let report = lattice_core::fuse::fuse(&manifest, &binary, false).unwrap();
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
        lattice_core::fuse::fuse(&manifest, &dir.0.join("absent"), false)
            .unwrap()
            .exit_code(),
        2
    );
}

// Requirement: Fuse exit codes
// Requirement: Tri-format output
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
            let strict = Command::new(env!("CARGO_BIN_EXE_lattice"))
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

// Requirement: Read a fuse profile
// Requirement: Suppression in program composition

const SOURCE_PROFILE: &str = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\n";

fn findings_json(report: &lattice_core::types::FuseReport) -> Vec<Value> {
    let value: Value = serde_json::from_str(&output_result(report, "json").unwrap()).unwrap();
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
    .expect_err("CONFIG_ERROR cannot be suppressed at the fuse level either");
    assert!(error.contains("CONFIG_ERROR"), "{error}");
    let error = load_fuse_profile(&dir.write(
        "typo.yaml",
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: VACANCY, node_id: [x]}\n",
    ))
    .expect_err("unknown keys are rejected");
    assert!(error.contains("node_id"), "{error}");
}

#[test]
fn suppress_source_level_suppression_survives_the_merge() {
    let dir = Scratch::new();
    dir.write("z.yaml", SOURCE_PROFILE);
    dir.write("a.yaml", SOURCE_PROFILE);
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([{
        "code": "UNTRACED", "severity": "error", "message": "m",
        "file": "x", "line": 1, "suppressed": true
    }]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", false);
    assert_eq!(
        report.exit_code(),
        0,
        "a source-suppressed error never gates"
    );
    let findings = findings_json(&report);
    let untraced = findings.iter().find(|f| f["code"] == "UNTRACED").unwrap();
    assert_eq!(untraced["suppressed"], true);
    assert_eq!(untraced["source"], "z");
    assert!(
        !findings.iter().any(|f| f["code"] == "SUPPRESS_UNUSED"),
        "the fuse profile declared nothing, so nothing is stale"
    );
    let plain = output_result(&report, "plain").unwrap();
    assert!(!plain.contains("UNTRACED"), "{plain}");
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
    for format in ["plain", "rich"] {
        let text = output_result(&report, format).unwrap();
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
    assert_eq!(unused.issue.severity, lattice_core::types::Severity::Info);
    assert_eq!(unused.issue.provenance.file, "<fuse-profile>");
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
    // A source's UNTRACED is that source's to suppress: the fuse entry matches
    // nothing the command itself collected, so it is stale, and the source
    // finding keeps gating.
    let dir = Scratch::new();
    dir.write("z.yaml", SOURCE_PROFILE);
    dir.write("a.yaml", SOURCE_PROFILE);
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([{
        "code": "UNTRACED", "severity": "error", "message": "m", "file": "x", "line": 1
    }]);
    let report = report(
        &dir,
        z,
        trace(json!([])),
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: UNTRACED}\n",
        false,
    );
    assert_eq!(report.exit_code(), 1);
    let findings = findings_json(&report);
    let untraced = findings.iter().find(|f| f["code"] == "UNTRACED").unwrap();
    assert!(untraced.get("suppressed").is_none());
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
    assert_eq!(vacancy.issue.severity, lattice_core::types::Severity::Error);
    assert!(vacancy.issue.suppressed);
}
