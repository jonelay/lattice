//! The `query` capability's scenarios, driven through the built binary.
//!
//! Queries produce no findings, so what these check is the two-valued exit
//! contract, the answer payloads, and where adapter issues land. The diff
//! scenarios build a real git repo per case. `mini-repo` is a fixture
//! directory, not a repository; diff's contract is about revisions.

mod common;

use std::path::Path;

use common::{Case, code, ingest, stderr, stdout};
use lattice_core::output::output_result;
use lattice_core::query::diff;
use lattice_core::types::DiffReport;

/// Three node kinds and three edge kinds, one of which (`mitigates`) the
/// register never instantiates. The zero-row scenario needs a declared kind
/// with no instances.
const QUERY_PROFILE: &str = r#"
name: q
profile_version: "1.0.0"
node_kinds:
  need:
    id_pattern: "^N-\\d+$"
  req:
    id_pattern: "^REQ-\\d+$"
  test:
    id_pattern: "^T-\\d+$"
edge_kinds:
  derives:
    allowed: [[req, need]]
  verifies:
    allowed: [[test, req]]
  mitigates:
    allowed: [[req, req]]
"#;

/// A small chain (T-1 verifies REQ-1 derives N-1) plus an orphan (N-2).
const GRAPH_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [
    {"id": "T-1", "kind": "test", "attrs": {"status": "active"}, "provenance": {"file": "t.py", "line": 1}},
    {"id": "REQ-1", "kind": "req", "attrs": {"status": "deferred"}, "provenance": {"file": "r.md", "line": 2}},
    {"id": "N-1", "kind": "need", "attrs": {"status": "active"}, "provenance": {"file": "n.md", "line": 3}},
    {"id": "N-2", "kind": "need", "attrs": {"status": "deferred"}, "provenance": {"file": "n.md", "line": 4}}],
  "edges": [
    {"src": "T-1", "tgt": "REQ-1", "kind": "verifies", "provenance": {"file": "t.py", "line": 1}},
    {"src": "REQ-1", "tgt": "N-1", "kind": "derives", "provenance": {"file": "r.md", "line": 2}}]}"#;

fn graph_case() -> Case {
    Case::with_profile(
        QUERY_PROFILE,
        &format!("cat <<'DOC'\n{GRAPH_DOCUMENT}\nDOC"),
    )
}

fn json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&stdout(output)).expect("stdout is JSON")
}

// Requirement: Counts query
// Requirement: Query subcommand family
// Requirement: Query output follows the tri-format contract

#[test]
fn counts_tallies_every_kind_including_declared_zero_rows() {
    let case = graph_case();
    let output = case.run(&["query", "counts", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    for line in [
        "node need 2",
        "node req 1",
        "node test 1",
        "edge derives 1",
        "edge verifies 1",
        "edge mitigates 0",
    ] {
        assert!(text.contains(line), "missing '{line}' in:\n{text}");
    }
}

#[test]
fn counts_json_carries_tallies_as_numbers() {
    let case = graph_case();
    let output = case.run(&["query", "counts", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value = json(&output);
    assert_eq!(value["nodes"]["need"], 2);
    assert_eq!(value["nodes"]["req"], 1);
    assert_eq!(value["edges"]["mitigates"], 0);
    assert_eq!(value["edges"]["verifies"], 1);
}

#[test]
fn counts_includes_a_kind_the_profile_does_not_declare() {
    // MINIMAL_PROFILE declares only `req`/`derives`; the register carries more.
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "W-1", "kind": "widget", "attrs": {},
                 "provenance": {"file": "w.md", "line": 1}}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["query", "counts", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("node widget 1"), "{text}");
    assert!(text.contains("node req 0"), "{text}");
}

// Requirement: Orphans query

#[test]
fn orphans_lists_only_nodes_with_no_edges() {
    let case = graph_case();
    let output = case.run(&["query", "orphans", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("N-2 need n.md:4"), "{text}");
    assert!(!text.contains("REQ-1"), "{text}");
    assert!(!text.contains("N-1 "), "{text}");
}

#[test]
fn orphans_kind_filter_restricts_the_answer() {
    let case = graph_case();
    let output = case.run(&["query", "orphans", "--kind", "req", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("N-2"), "{text}");
    assert!(text.contains("No orphans."), "{text}");
}

#[test]
fn orphans_attribute_filter_restricts_the_answer() {
    let case = graph_case();
    let output = case.run(&[
        "query",
        "orphans",
        "--filter",
        "status=active",
        "--format",
        "plain",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No orphans."),
        "{}",
        stdout(&output)
    );
}

#[test]
fn orphans_unknown_kind_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "orphans", "--kind", "widget"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("widget"), "{}", stderr(&output));
}

// An edge counts for a node whenever it names that node's ID. The node is
// referenced, so it is not standing alone, whatever became of the far end.
#[test]
fn a_node_named_by_an_edge_with_an_undeclared_far_endpoint_is_not_an_orphan() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "r.md", "line": 1}}],
      "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
                 "provenance": {"file": "r.md", "line": 1}}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["query", "orphans", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No orphans."),
        "{}",
        stdout(&output)
    );
}

// Requirement: Reachability queries

#[test]
fn reaches_reports_the_transitive_closure_sorted_by_id() {
    let case = graph_case();
    let output = case.run(&["query", "reaches", "T-1", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    let n1 = text.find("N-1 need").expect(&text);
    let req = text.find("REQ-1 req").expect(&text);
    assert!(n1 < req, "not sorted by ID:\n{text}");
    assert!(!text.contains("T-1 test"), "origin reported:\n{text}");
}

#[test]
fn reaches_edge_kind_restriction_cuts_the_closure() {
    let case = graph_case();
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--edge-kind",
        "verifies",
        "--format",
        "plain",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("REQ-1 req"), "{text}");
    assert!(!text.contains("N-1"), "{text}");
}

#[test]
fn reaches_attribute_filter_does_not_cut_the_traversal() {
    let case = graph_case();
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--filter",
        "status=active",
        "--format",
        "plain",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("N-1 need"), "{text}");
    assert!(!text.contains("REQ-1 req"), "{text}");
}

#[test]
fn invalid_attribute_filter_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "reaches", "T-1", "--filter", "status"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("invalid filter"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn reached_by_walks_incoming_edges() {
    let case = graph_case();
    let output = case.run(&["query", "reached-by", "N-1", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("REQ-1 req"), "{text}");
    assert!(text.contains("T-1 test"), "{text}");
}

#[test]
fn reaches_unknown_id_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "reaches", "REQ-99"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("REQ-99"), "{}", stderr(&output));
}

#[test]
fn reaches_unknown_edge_kind_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "reaches", "T-1", "--edge-kind", "nope"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("nope"), "{}", stderr(&output));
}

#[test]
fn an_undeclared_endpoint_is_never_reported_as_reached() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "r.md", "line": 1}}],
      "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
                 "provenance": {"file": "r.md", "line": 1}}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["query", "reaches", "REQ-1", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(!stdout(&output).contains("REQ-9"), "{}", stdout(&output));
}

/// A document over the query profile's kinds with the given edges; every
/// endpoint named in `declared` is a node, anything else is a vacancy.
fn edges_document(declared: &[(&str, &str)], edges: &[(&str, &str, &str)]) -> String {
    let nodes = declared
        .iter()
        .map(|(id, kind)| {
            format!(r#"{{"id": "{id}", "kind": "{kind}", "attrs": {{}}, "provenance": {{"file": "f", "line": 1}}}}"#)
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let edges = edges
        .iter()
        .map(|(src, tgt, kind)| {
            format!(r#"{{"src": "{src}", "tgt": "{tgt}", "kind": "{kind}", "provenance": {{"file": "f", "line": 1}}}}"#)
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(r#"{{"interface_version": "1.0", "nodes": [{nodes}], "edges": [{edges}]}}"#)
}

fn reach_case(declared: &[(&str, &str)], edges: &[(&str, &str, &str)]) -> Case {
    let document = edges_document(declared, edges);
    Case::with_profile(QUERY_PROFILE, &format!("cat <<'DOC'\n{document}\nDOC"))
}

/// `id → tainted` for every node in a `--check-resolved` JSON answer.
fn tainted_by_id(output: &std::process::Output) -> Vec<(String, Option<bool>)> {
    json(output)["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|n| {
            (
                n["id"].as_str().unwrap().to_string(),
                n.get("tainted")
                    .map(|t| t.as_bool().expect("tainted is a bool")),
            )
        })
        .collect()
}

fn owned(pairs: &[(&str, Option<bool>)]) -> Vec<(String, Option<bool>)> {
    pairs.iter().map(|(id, t)| (id.to_string(), *t)).collect()
}

#[test]
fn check_resolved_reach_through_declared_nodes_is_clean() {
    let case = graph_case();
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        tainted_by_id(&output),
        owned(&[("N-1", Some(false)), ("REQ-1", Some(false))])
    );
}

#[test]
fn check_resolved_reach_through_a_vacancy_is_tainted() {
    let case = reach_case(
        &[("T-1", "test"), ("N-1", "need")],
        &[("T-1", "REQ-9", "verifies"), ("REQ-9", "N-1", "derives")],
    );
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(tainted_by_id(&output), owned(&[("N-1", Some(true))]));
}

#[test]
fn check_resolved_reach_taint_carries_past_the_first_declared_node() {
    let case = reach_case(
        &[("T-1", "test"), ("REQ-1", "req"), ("N-1", "need")],
        &[
            ("T-1", "REQ-9", "verifies"),
            ("REQ-9", "REQ-1", "mitigates"),
            ("REQ-1", "N-1", "derives"),
        ],
    );
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        tainted_by_id(&output),
        owned(&[("N-1", Some(true)), ("REQ-1", Some(true))])
    );
}

#[test]
fn check_resolved_reach_with_one_clean_path_is_not_tainted() {
    let case = reach_case(
        &[("T-1", "test"), ("REQ-1", "req"), ("N-1", "need")],
        &[
            ("T-1", "REQ-1", "verifies"),
            ("REQ-1", "N-1", "derives"),
            ("T-1", "REQ-9", "verifies"),
            ("REQ-9", "N-1", "derives"),
        ],
    );
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        tainted_by_id(&output),
        owned(&[("N-1", Some(false)), ("REQ-1", Some(false))])
    );
}

/// The vacancy path is the shorter one, so a breadth-first walk meets N-1
/// tainted (and expands its descendant N-3) before the longer clean path
/// arrives. The answer must still be clean for both.
#[test]
fn check_resolved_reach_clean_path_found_later_wins() {
    let case = reach_case(
        &[
            ("T-1", "test"),
            ("REQ-1", "req"),
            ("REQ-2", "req"),
            ("REQ-3", "req"),
            ("N-1", "need"),
            ("N-3", "need"),
        ],
        &[
            ("T-1", "REQ-9", "verifies"),
            ("REQ-9", "N-1", "derives"),
            ("N-1", "N-3", "derives"),
            ("T-1", "REQ-1", "verifies"),
            ("REQ-1", "REQ-2", "mitigates"),
            ("REQ-2", "REQ-3", "mitigates"),
            ("REQ-3", "N-1", "derives"),
        ],
    );
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        tainted_by_id(&output),
        owned(&[
            ("N-1", Some(false)),
            ("N-3", Some(false)),
            ("REQ-1", Some(false)),
            ("REQ-2", Some(false)),
            ("REQ-3", Some(false))
        ])
    );
}

#[test]
fn check_resolved_is_not_an_orphans_flag() {
    let case = graph_case();
    let output = case.run(&["query", "orphans", "--check-resolved"]);
    assert_eq!(code(&output), 2, "{}", stdout(&output));
}

#[test]
fn check_resolved_reach_never_reports_the_vacancy_itself() {
    let case = reach_case(&[("T-1", "test")], &[("T-1", "REQ-9", "verifies")]);
    let output = case.run(&[
        "query",
        "reaches",
        "T-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(tainted_by_id(&output), vec![]);
}

#[test]
fn check_resolved_reached_by_walks_incoming_edges() {
    let case = reach_case(
        &[("T-1", "test"), ("N-1", "need")],
        &[("T-1", "REQ-9", "verifies"), ("REQ-9", "N-1", "derives")],
    );
    let output = case.run(&[
        "query",
        "reached-by",
        "N-1",
        "--check-resolved",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(tainted_by_id(&output), owned(&[("T-1", Some(true))]));
}

#[test]
fn check_resolved_reach_marks_tainted_nodes_in_plain_and_rich() {
    let case = reach_case(
        &[("T-1", "test"), ("REQ-1", "req"), ("N-1", "need")],
        &[
            ("T-1", "REQ-1", "verifies"),
            ("T-1", "REQ-9", "verifies"),
            ("REQ-9", "N-1", "derives"),
        ],
    );
    for format in ["plain", "rich"] {
        let output = case.run(&[
            "query",
            "reaches",
            "T-1",
            "--check-resolved",
            "--format",
            format,
        ]);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        let text = stdout(&output);
        let line = |id: &str| {
            text.lines()
                .find(|l| l.starts_with(&format!("{id} ")))
                .unwrap_or_else(|| panic!("{format}: no line for {id}:\n{text}"))
        };
        assert!(line("N-1").ends_with("(tainted)"), "{format}:\n{text}");
        assert!(!line("REQ-1").ends_with("(tainted)"), "{format}:\n{text}");
    }
}

#[test]
fn reach_without_check_resolved_carries_no_tainted_field() {
    let case = reach_case(
        &[("T-1", "test"), ("N-1", "need")],
        &[("T-1", "REQ-9", "verifies"), ("REQ-9", "N-1", "derives")],
    );
    let output = case.run(&["query", "reaches", "T-1", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(tainted_by_id(&output), owned(&[("N-1", None)]));
}

// Requirement: Path query

#[test]
fn path_reports_the_connecting_chain() {
    let case = graph_case();
    let output = case.run(&["query", "path", "T-1", "N-1", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("T-1 -[verifies]-> REQ-1 -[derives]-> N-1"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn no_path_is_an_answer_not_a_failure() {
    let case = graph_case();
    let output = case.run(&["query", "path", "N-1", "T-1", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No path from N-1 to T-1."),
        "{}",
        stdout(&output)
    );
}

#[test]
fn path_json_carries_nodes_and_edges() {
    let case = graph_case();
    let output = case.run(&["query", "path", "T-1", "N-1", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value = json(&output);
    assert_eq!(value["found"], true);
    assert_eq!(value["nodes"], serde_json::json!(["T-1", "REQ-1", "N-1"]));
    assert_eq!(value["edges"], serde_json::json!(["verifies", "derives"]));
}

#[test]
fn path_unknown_endpoint_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "path", "T-1", "N-99"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("N-99"), "{}", stderr(&output));
}

// Requirement: Query exit codes are two-valued

#[test]
fn error_severity_adapter_issues_reach_stderr_but_not_the_exit_code() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "r.md", "line": 1}}],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "bad row",
                  "provenance": {"file": "r.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["query", "counts", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("node req 1"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("PARSE_ERROR"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_broken_adapter_is_still_exit_two_for_query() {
    let case = Case::running("exit 3");
    let output = case.run(&["query", "counts"]);
    assert_eq!(code(&output), 2);
}

// Requirement: Live two-revision diff

/// Run git in `dir`, with identity supplied inline so no host config is needed.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The register at revision A: a chain of two nodes, one pathway at M0.
const DIFF_DOCUMENT_A: &str = r#"{"interface_version": "1.0",
  "nodes": [
    {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 2}},
    {"id": "N-1", "kind": "need", "attrs": {"text": "a"}, "provenance": {"file": "n.md", "line": 3}}],
  "edges": [
    {"src": "REQ-1", "tgt": "N-1", "kind": "derives", "provenance": {"file": "r.md", "line": 2}}],
  "pathways": [{"name": "phase", "order": ["M0", "M1"], "current": "M0"}]}"#;

/// Revision B: REQ-1 merely moved lines, N-1's attrs changed, T-1 and its
/// edge arrived, and the pathway advanced.
const DIFF_DOCUMENT_B: &str = r#"{"interface_version": "1.0",
  "nodes": [
    {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 7}},
    {"id": "N-1", "kind": "need", "attrs": {"text": "b"}, "provenance": {"file": "n.md", "line": 3}},
    {"id": "T-1", "kind": "test", "attrs": {}, "provenance": {"file": "t.py", "line": 1}}],
  "edges": [
    {"src": "REQ-1", "tgt": "N-1", "kind": "derives", "provenance": {"file": "r.md", "line": 7}},
    {"src": "T-1", "tgt": "REQ-1", "kind": "verifies", "provenance": {"file": "t.py", "line": 1}}],
  "pathways": [{"name": "phase", "order": ["M0", "M1"], "current": "M1"}]}"#;

/// A case whose target is a git repo holding `doc.json` at two commits, with
/// an adapter that emits whatever `doc.json` the materialized target carries.
fn diff_case() -> (Case, String, String) {
    let case = Case::with_profile(QUERY_PROFILE, "cat \"$4/doc.json\"");
    git(&case.dir, &["init", "-q"]);
    std::fs::write(case.dir.join("doc.json"), DIFF_DOCUMENT_A).unwrap();
    git(&case.dir, &["add", "doc.json"]);
    git(&case.dir, &["commit", "-q", "-m", "a"]);
    let rev_a = git(&case.dir, &["rev-parse", "HEAD"]);
    std::fs::write(case.dir.join("doc.json"), DIFF_DOCUMENT_B).unwrap();
    git(&case.dir, &["commit", "-q", "-a", "-m", "b"]);
    let rev_b = git(&case.dir, &["rev-parse", "HEAD"]);
    (case, rev_a, rev_b)
}

#[test]
fn a_revision_diffed_against_itself_is_empty() {
    let (case, rev_a, _) = diff_case();
    let output = case.run(&["query", "diff", &rev_a, &rev_a, "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No differences."),
        "{}",
        stdout(&output)
    );
}

#[test]
fn diff_reports_additions_changes_and_the_pathway_but_not_a_move() {
    let (case, rev_a, rev_b) = diff_case();
    let output = case.run(&["query", "diff", &rev_a, &rev_b, "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    for line in [
        "node added T-1 test",
        "node changed N-1 need",
        "edge added T-1 verifies REQ-1",
        "pathway changed phase",
    ] {
        assert!(text.contains(line), "missing '{line}' in:\n{text}");
    }
    // REQ-1 and its edge only moved lines; provenance is excluded from identity.
    assert!(!text.contains("REQ-1 req"), "{text}");
    assert!(!text.contains("edge added REQ-1"), "{text}");
    assert!(!text.contains("edge removed"), "{text}");
}

#[test]
fn diff_json_carries_the_report_structured() {
    let (case, rev_a, rev_b) = diff_case();
    let output = case.run(&["query", "diff", &rev_a, &rev_b, "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value = json(&output);
    assert_eq!(value["nodes_added"][0]["id"], "T-1");
    assert_eq!(value["nodes_changed"][0]["id"], "N-1");
    assert_eq!(value["nodes_changed"][0]["attrs_a"]["text"], "a");
    assert_eq!(value["nodes_changed"][0]["attrs_b"]["text"], "b");
    assert_eq!(value["edges_added"][0]["src"], "T-1");
    assert_eq!(value["edges_changed"], serde_json::json!([]));
    assert_eq!(value["pathways_changed"][0], "phase");
    assert_eq!(value["nodes_removed"], serde_json::json!([]));
}

// An adapter may embed the absolute target path in `file` attrs, so two
// materialization directories would make every such node "changed" in a
// self-diff. Both revisions must land at the same path.
#[test]
fn a_target_path_embedded_in_attrs_does_not_diff() {
    let case = Case::with_profile(
        QUERY_PROFILE,
        r#"cat <<DOC
{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {"file": "$4/r.md"},
             "provenance": {"file": "$4/r.md", "line": 1}}]}
DOC"#,
    );
    git(&case.dir, &["init", "-q"]);
    std::fs::write(case.dir.join("r.md"), "r").unwrap();
    git(&case.dir, &["add", "r.md"]);
    git(&case.dir, &["commit", "-q", "-m", "a"]);
    let rev = git(&case.dir, &["rev-parse", "HEAD"]);

    let output = case.run(&["query", "diff", &rev, &rev, "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No differences."),
        "{}",
        stdout(&output)
    );
}

#[test]
fn diff_unknown_revision_exits_two() {
    let (case, rev_a, _) = diff_case();
    let output = case.run(&["query", "diff", &rev_a, "deadbeef"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("deadbeef"), "{}", stderr(&output));
}

#[test]
fn diff_against_a_target_that_is_not_a_git_repo_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "diff", "HEAD~1", "HEAD"]);
    assert_eq!(code(&output), 2);
}

#[test]
fn diff_leaves_the_target_working_tree_untouched() {
    let (case, rev_a, rev_b) = diff_case();
    let content_before = std::fs::read_to_string(case.dir.join("doc.json")).unwrap();
    let status_before = git(&case.dir, &["status", "--porcelain"]);
    let output = case.run(&["query", "diff", &rev_a, &rev_b]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let content_after = std::fs::read_to_string(case.dir.join("doc.json")).unwrap();
    assert_eq!(content_before, content_after);
    assert_eq!(status_before, git(&case.dir, &["status", "--porcelain"]));
}

// Requirement: Per-kind orphan exemption

#[test]
fn orphans_reports_an_orphan_ok_node_the_validator_exempts() {
    // orphan_ok is validation policy; the query answers the ask-time fact.
    let profile = QUERY_PROFILE.replace(
        "  need:\n    id_pattern: \"^N-\\\\d+$\"",
        "  need:\n    id_pattern: \"^N-\\\\d+$\"\n    orphan_ok: true",
    );
    assert_ne!(profile, QUERY_PROFILE, "the replacement must have applied");
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{GRAPH_DOCUMENT}\nDOC"));
    let output = case.run(&["query", "orphans", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("N-2"), "{}", stdout(&output));
}

// Requirement: Provenance query

#[test]
fn at_reports_the_entries_declared_at_a_path() {
    let case = graph_case();
    let output = case.run(&["query", "at", "r.md", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("REQ-1"), "{text}");
    assert!(!text.contains("T-1"), "{text}");
    assert!(!text.contains("N-2"), "{text}");
}

#[test]
fn at_a_missing_register_file_stays_queryable() {
    // The provenance path exists nowhere on disk; the issue names it, which
    // is exactly when the answer matters most.
    let document = r#"{"interface_version": "1.0", "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR",
                  "message": "REQUIREMENTS.md not found at gone.md",
                  "provenance": {"file": "gone.md", "line": 0}, "node_id": null}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["query", "at", "gone.md", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("PARSE_ERROR"), "{text}");
    assert!(text.contains("gone.md"), "{text}");
}

#[test]
fn at_an_unknown_path_exits_two() {
    let case = graph_case();
    let output = case.run(&["query", "at", "nope.md"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("nope.md"), "{}", stderr(&output));
}

#[test]
fn at_an_existing_file_with_no_entries_is_an_empty_answer() {
    let case = graph_case();
    std::fs::write(case.dir.join("empty.md"), "").unwrap();
    let output = case.run(&["query", "at", "empty.md", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No entries or findings"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn at_a_directory_argument_matches_the_files_beneath_it() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "specs/a.md", "line": 1}}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["query", "at", "specs", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("REQ-1"), "{}", stdout(&output));
}

#[test]
fn at_a_relative_argument_matches_target_joined_provenance() {
    // The heredoc is unquoted so $4 (the adapter's --target value) lands in
    // the provenance, the way real adapters compose absolute paths.
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "$4/r.md", "line": 1}}]}"#;
    let case = Case::running(&format!("cat <<DOC\n{document}\nDOC"));
    let output = case.run(&["query", "at", "r.md", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("REQ-1"), "{}", stdout(&output));
}

#[test]
fn at_a_finding_at_the_path_attached_to_an_entry_elsewhere_is_listed() {
    // The dangling edge is written in r.md but attaches to T-1, whose entry
    // lives at t.py. Querying r.md must still surface the finding.
    let document = r#"{"interface_version": "1.0",
      "nodes": [
        {"id": "T-1", "kind": "test", "attrs": {}, "provenance": {"file": "t.py", "line": 1}}],
      "edges": [
        {"src": "T-1", "tgt": "REQ-9", "kind": "verifies", "provenance": {"file": "r.md", "line": 9}}]}"#;
    let case = Case::with_profile(QUERY_PROFILE, &format!("cat <<'DOC'\n{document}\nDOC"));
    let output = case.run(&["query", "at", "r.md", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("VACANCY"), "{text}");
    assert!(!text.contains("t.py"), "{text}");
}

#[test]
fn at_json_carries_entries_and_findings() {
    let case = graph_case();
    let output = case.run(&["query", "at", "r.md", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let value = json(&output);
    assert_eq!(value["path"], "r.md");
    assert_eq!(value["entries"][0]["id"], "REQ-1");
    assert_eq!(value["entries"].as_array().unwrap().len(), 1);
    assert_eq!(value["entries"][0]["edges"][0]["tgt"], "N-1");
    assert_eq!(value["entries"][0]["edges"][0]["kind"], "derives");
    assert!(value["findings"].is_array(), "{value}");
}

#[test]
fn at_an_absolute_argument_matches_relative_provenance() {
    // GRAPH_DOCUMENT writes target-relative provenance; the absolute form of
    // the same file must not read as a clean empty answer.
    let case = graph_case();
    let abs = case.dir.join("r.md");
    let output = case.run(&["query", "at", abs.to_str().unwrap(), "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("REQ-1"), "{}", stdout(&output));
}

#[test]
fn at_an_absolute_directory_argument_matches_relative_provenance_beneath_it() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "specs/a.md", "line": 1}}]}"#;
    let case = Case::emitting(document);
    let abs = case.dir.join("specs");
    let output = case.run(&["query", "at", abs.to_str().unwrap(), "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("REQ-1"), "{}", stdout(&output));
}

// Requirement: Live two-revision diff

/// Two registers that differ only in what the library-level diff is asked
/// about, so each scenario reads as its two edge lists.
fn diff_edges(edges_a: serde_json::Value, edges_b: serde_json::Value) -> DiffReport {
    let document = |edges| {
        ingest(serde_json::json!({
            "interface_version": "1.0",
            "nodes": [
                {"id": "A", "kind": "req", "attrs": {}, "provenance": {"file": "r", "line": 1}},
                {"id": "B", "kind": "req", "attrs": {}, "provenance": {"file": "r", "line": 2}},
                {"id": "C", "kind": "req", "attrs": {}, "provenance": {"file": "r", "line": 3}}],
            "edges": edges,
        }))
    };
    diff("a", &document(edges_a), "b", &document(edges_b))
}

fn edge(src: &str, tgt: &str, attrs: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"src": src, "tgt": tgt, "kind": "derives", "attrs": attrs,
                       "provenance": {"file": "r", "line": 1}})
}

#[test]
fn an_edge_whose_attrs_changed_is_reported_as_changed_with_both_sides() {
    let report = diff_edges(
        serde_json::json!([edge("A", "B", serde_json::json!({"confidence": "low"}))]),
        serde_json::json!([edge("A", "B", serde_json::json!({"confidence": "high"}))]),
    );
    assert!(report.edges_added.is_empty() && report.edges_removed.is_empty());
    assert_eq!(report.edges_changed.len(), 1);
    let changed = &report.edges_changed[0];
    assert_eq!(
        (
            changed.src.as_str(),
            changed.tgt.as_str(),
            changed.kind.as_str()
        ),
        ("A", "B", "derives")
    );
    assert_eq!(changed.attrs_a["confidence"], "low");
    assert_eq!(changed.attrs_b["confidence"], "high");
}

#[test]
fn an_edge_that_moved_to_another_target_is_removed_and_added_not_changed() {
    let report = diff_edges(
        serde_json::json!([edge("A", "B", serde_json::json!({}))]),
        serde_json::json!([edge("A", "C", serde_json::json!({}))]),
    );
    assert!(report.edges_changed.is_empty());
    assert_eq!(report.edges_removed[0].tgt, "B");
    assert_eq!(report.edges_added[0].tgt, "C");
}

#[test]
fn parallel_edges_match_pairwise_in_document_order() {
    let report = diff_edges(
        serde_json::json!([
            edge("A", "B", serde_json::json!({"w": 1})),
            edge("A", "B", serde_json::json!({"w": 2}))
        ]),
        serde_json::json!([
            edge("A", "B", serde_json::json!({"w": 1})),
            edge("A", "B", serde_json::json!({"w": 3})),
            edge("A", "B", serde_json::json!({"w": 4}))
        ]),
    );
    assert_eq!(report.edges_changed.len(), 1, "{:?}", report.edges_changed);
    assert_eq!(report.edges_changed[0].attrs_a["w"], 2);
    assert_eq!(report.edges_changed[0].attrs_b["w"], 3);
    assert_eq!(report.edges_added.len(), 1, "the third edge has no partner");
    assert!(report.edges_removed.is_empty());
}

#[test]
fn identical_edge_attrs_do_not_diff() {
    let edges = serde_json::json!([edge("A", "B", serde_json::json!({"w": 1}))]);
    let report = diff_edges(edges.clone(), edges);
    assert!(report.is_empty());
}

#[test]
fn a_changed_edge_renders_in_every_format() {
    let report = diff_edges(
        serde_json::json!([edge("A", "B", serde_json::json!({"w": 1}))]),
        serde_json::json!([edge("A", "B", serde_json::json!({"w": 2}))]),
    );
    assert_eq!(
        output_result(&report, "plain").unwrap(),
        "edge changed A derives B"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&output_result(&report, "json").unwrap()).unwrap();
    assert_eq!(
        parsed["edges_changed"],
        serde_json::json!([{"attrs_a": {"w": 1}, "attrs_b": {"w": 2},
                            "kind": "derives", "src": "A", "tgt": "B"}])
    );
    assert!(
        output_result(&report, "rich")
            .unwrap()
            .contains("edge changed A derives B")
    );
}
