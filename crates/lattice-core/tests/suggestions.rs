//! The suggestion overlay: document schema, version negotiation, ordering,
//! and the property the whole feature rests on: advice never gates.

mod common;

use std::path::{Path, PathBuf};

use common::{Case, MINIMAL_PROFILE};
use serde_json::json;

/// A interface document declaring two `req` nodes the suggestions can name.
/// Both edges run in both directions so neither node is UNREFERENCED or UNTRACED.
const TWO_NODES: &str = r#"{
  "interface_version": "1.1",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "reqs.md", "line": 3}},
            {"id": "REQ-2", "kind": "req", "attrs": {},
             "provenance": {"file": "reqs.md", "line": 9}}],
  "edges": [{"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
             "provenance": {"file": "reqs.md", "line": 3}},
            {"src": "REQ-2", "tgt": "REQ-1", "kind": "derives",
             "provenance": {"file": "reqs.md", "line": 9}}]
}"#;

fn suggestion(src: &str, tgt: &str, score: f64, basis: &str) -> serde_json::Value {
    json!({"src": src, "tgt": tgt, "kind": "derives", "score": score, "basis": basis})
}

fn document(entries: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "suggestion_version": "1.0",
        "producer": "lattice-suggest 0.1.0",
        "suggestions": entries,
    })
}

/// Write a suggestion document into the case's directory and return its path.
fn write_doc(case: &Case, document: &serde_json::Value) -> PathBuf {
    let path = case.dir.join("suggestions.json");
    std::fs::write(&path, serde_json::to_string(document).unwrap()).unwrap();
    path
}

/// Run `validate` over `TWO_NODES` with the given suggestion document.
fn run_with(document: &serde_json::Value, extra: &[&str]) -> (String, String, Option<i32>) {
    let case = Case::emitting(TWO_NODES);
    let path = write_doc(&case, document);
    let mut args = vec![
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let output = case.run(&args);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// Requirement: Suggestion document schema

#[test]
fn a_document_matching_the_schema_renders() {
    let (stdout, _, code) = run_with(
        &document(vec![suggestion("REQ-1", "REQ-2", 0.83, "cosine 0.83")]),
        &[],
    );

    assert!(stdout.contains("SUGGESTED_EDGE"), "{stdout}");
    assert!(stdout.contains("REQ-1"), "{stdout}");
    assert_eq!(code, Some(0));
}

#[test]
fn a_suggestion_missing_a_field_fails_the_schema() {
    let entry = json!({"src": "REQ-1", "tgt": "REQ-2", "kind": "derives", "score": 0.5});
    let (stdout, stderr, code) = run_with(&document(vec![entry]), &[]);

    assert_eq!(code, Some(2), "stdout={stdout} stderr={stderr}");
    assert!(stderr.contains("basis"), "{stderr}");
    assert!(!stdout.contains("SUGGESTED_EDGE"), "{stdout}");
}

#[test]
fn a_non_numeric_score_fails_the_schema() {
    let entry = json!({"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
                       "score": "high", "basis": "b"});
    let (_, stderr, code) = run_with(&document(vec![entry]), &[]);

    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("score"), "{stderr}");
}

#[test]
fn the_schema_carries_basis_text_verbatim() {
    let basis = "cosine 0.8312; model nomic-embed-text-v1.5";
    let (stdout, _, _) = run_with(
        &document(vec![suggestion("REQ-1", "REQ-2", 0.83, basis)]),
        &[],
    );

    assert!(stdout.contains(basis), "{stdout}");
}

#[test]
fn the_schema_names_its_producer() {
    let (stdout, _, _) = run_with(
        &document(vec![suggestion("REQ-1", "REQ-2", 0.83, "cosine")]),
        &[],
    );

    assert!(stdout.contains("lattice-suggest 0.1.0"), "{stdout}");
}

// Requirement: Suggestion document is versioned and negotiated

#[test]
fn an_unsupported_suggestion_version_is_exit_two() {
    let mut doc = document(vec![suggestion("REQ-1", "REQ-2", 0.5, "b")]);
    doc["suggestion_version"] = json!("9.9");
    let (_, stderr, code) = run_with(&doc, &[]);

    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("9.9"), "{stderr}");
}

/// A broken overlay must stay distinguishable from a clean run and from a real
/// finding; the three-valued exit code is what callers read to tell them apart.
#[test]
fn a_version_mismatch_on_a_clean_register_is_not_zero_or_one() {
    let mut doc = document(vec![]);
    doc["suggestion_version"] = json!("9.9");
    let (_, _, code) = run_with(&doc, &[]);

    assert_eq!(code, Some(2));
}

#[test]
fn a_missing_suggestion_version_is_exit_two() {
    let doc = json!({"producer": "p", "suggestions": []});
    let (_, stderr, code) = run_with(&doc, &[]);

    assert_eq!(code, Some(2), "{stderr}");
}

// Requirement: Suggestions are ordered deterministically in the document

/// The report carries one total order across every finding, so rank does not
/// survive into it. It does not need to: the score is on every line, and the
/// document is the ranked artifact.
#[test]
fn report_ordering_leaves_the_score_on_every_suggestion() {
    let (stdout, _, _) = run_with(
        &document(vec![
            suggestion("REQ-1", "REQ-2", 0.9, "first"),
            suggestion("REQ-2", "REQ-1", 0.7, "second"),
            suggestion("REQ-1", "REQ-2", 0.4, "third"),
        ]),
        &[],
    );

    for line in stdout.lines().filter(|l| l.contains("SUGGESTED_EDGE")) {
        assert!(line.contains("score 0."), "{line}");
    }
    for score in ["0.9000", "0.7000", "0.4000"] {
        assert!(stdout.contains(score), "{score} in {stdout}");
    }
}

#[test]
fn suggestion_ordering_is_deterministic_across_runs() {
    let doc = document(vec![
        suggestion("REQ-1", "REQ-2", 0.9, "first"),
        suggestion("REQ-2", "REQ-1", 0.7, "second"),
    ]);
    let case = Case::emitting(TWO_NODES);
    let path = write_doc(&case, &doc);
    let args = [
        "validate",
        "--format",
        "json",
        "--suggestions",
        path.to_str().unwrap(),
    ];

    assert_eq!(case.run(&args).stdout, case.run(&args).stdout);
}

/// The overlay adds findings; it never rewrites the ones the register produced.
#[test]
fn overlay_ordering_leaves_the_registers_findings_unchanged() {
    // A lone node with no edges is unreferenced and untraced, so this register
    // has findings of its own for the overlay to sort in among.
    let lone = r#"{
      "interface_version": "1.1",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "reqs.md", "line": 3}}]
    }"#;
    let case = Case::with_profile(MINIMAL_PROFILE, &format!("cat <<'DOC'\n{lone}\nDOC"));
    let without = case.run(&["validate", "--format", "plain"]);
    let path = write_doc(
        &case,
        &document(vec![suggestion("REQ-1", "REQ-1", 0.9, "b")]),
    );
    let with = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ]);

    let without = String::from_utf8_lossy(&without.stdout).into_owned();
    let with = String::from_utf8_lossy(&with.stdout).into_owned();
    let kept: Vec<&str> = with
        .lines()
        .filter(|l| !l.contains("SUGGESTED_EDGE"))
        .collect();

    assert!(
        !without.trim().is_empty(),
        "the register must have findings"
    );
    assert_eq!(kept.join("\n"), without.trim_end(), "{with}");
}

// Requirement: Suggestions flag

#[test]
fn the_flag_takes_several_documents() {
    let case = Case::emitting(TWO_NODES);
    let first = case.dir.join("a.json");
    let second = case.dir.join("b.json");
    let mut doc_a = document(vec![suggestion("REQ-1", "REQ-2", 0.9, "from-a")]);
    doc_a["producer"] = json!("producer-a");
    let mut doc_b = document(vec![suggestion("REQ-2", "REQ-1", 0.8, "from-b")]);
    doc_b["producer"] = json!("producer-b");
    std::fs::write(&first, serde_json::to_string(&doc_a).unwrap()).unwrap();
    std::fs::write(&second, serde_json::to_string(&doc_b).unwrap()).unwrap();

    let output = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        first.to_str().unwrap(),
        "--suggestions",
        second.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("from-a"), "{stdout}");
    assert!(stdout.contains("from-b"), "{stdout}");
    assert!(
        stdout.contains("producer-a") && stdout.contains("producer-b"),
        "{stdout}"
    );
}

/// The overlay adds to a report and never alters the rest of it.
#[test]
fn the_absent_flag_changes_nothing() {
    let case = Case::emitting(TWO_NODES);
    let bare = case.run(&["validate", "--format", "plain"]);
    let path = write_doc(&case, &document(vec![]));
    let empty = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ]);

    assert_eq!(bare.stdout, empty.stdout);
    assert_eq!(bare.status.code(), empty.status.code());
}

// Requirement: The suggestions overlay never changes the exit code

#[test]
fn suggestions_on_a_clean_register_exit_zero() {
    for extra in [vec![], vec!["--strict"]] {
        let (stdout, _, code) = run_with(
            &document(vec![suggestion("REQ-1", "REQ-2", 0.9, "b")]),
            &extra,
        );
        assert_eq!(code, Some(0), "extra={extra:?} stdout={stdout}");
    }
}

#[test]
fn suggestions_do_not_change_an_exit_code_findings_already_set() {
    // An undeclared node kind is an error-severity finding, so this register
    // exits 1 with or without the overlay.
    let broken = r#"{
      "interface_version": "1.1",
      "nodes": [{"id": "REQ-1", "kind": "mystery", "attrs": {},
                 "provenance": {"file": "reqs.md", "line": 3}}]
    }"#;
    let case = Case::with_profile(MINIMAL_PROFILE, &format!("cat <<'DOC'\n{broken}\nDOC"));
    let bare = case.run(&["validate", "--format", "plain"]);
    // A rendered suggestion and an unresolved one, so this exercises both
    // overlay codes rather than just the flag's presence.
    let path = write_doc(
        &case,
        &document(vec![
            suggestion("REQ-1", "REQ-1", 0.9, "resolvable"),
            suggestion("REQ-1", "REQ-404", 0.8, "stale"),
        ]),
    );
    let with = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&with.stdout);

    assert!(stdout.contains("SUGGESTED_EDGE"), "{stdout}");
    assert!(stdout.contains("SUGGESTION_UNRESOLVED"), "{stdout}");
    assert_eq!(bare.status.code(), Some(1));
    assert_eq!(with.status.code(), Some(1));
}

// Requirement: An unresolvable suggestion is reported, never dropped

#[test]
fn an_unresolved_target_id_is_reported_and_the_rest_still_render() {
    let (stdout, _, code) = run_with(
        &document(vec![
            suggestion("REQ-1", "REQ-404", 0.9, "stale"),
            suggestion("REQ-1", "REQ-2", 0.8, "live"),
        ]),
        &[],
    );

    assert!(stdout.contains("SUGGESTION_UNRESOLVED"), "{stdout}");
    assert!(stdout.contains("REQ-404"), "{stdout}");
    assert!(stdout.contains("live"), "{stdout}");
    assert_eq!(code, Some(0));
}

/// Silence would read as "this ranker had nothing to say", which is the one
/// answer the tool must never give.
#[test]
fn an_entirely_unresolved_document_reports_every_entry() {
    let (stdout, _, code) = run_with(
        &document(vec![
            suggestion("REQ-900", "REQ-901", 0.9, "a"),
            suggestion("REQ-902", "REQ-903", 0.8, "b"),
        ]),
        &[],
    );

    assert_eq!(
        stdout.matches("SUGGESTION_UNRESOLVED").count(),
        2,
        "{stdout}"
    );
    assert_eq!(code, Some(0));
}

// Requirement: A suggestion document is ephemeral

/// Scoped to the target repo and the document's own directory: a validate run
/// legitimately writes a scratch profile under the system temp dir, and an
/// unscoped assertion would fail on behaviour that predates the overlay.
#[test]
fn a_readonly_run_with_suggestions_writes_nothing() {
    let case = Case::emitting(TWO_NODES);
    let path = write_doc(
        &case,
        &document(vec![suggestion("REQ-1", "REQ-2", 0.9, "b")]),
    );

    let before = snapshot(&case.dir);
    let output = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ]);
    let after = snapshot(&case.dir);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(before, after, "the run created or modified a file");
}

/// Every file under `root`, with its length and modification time.
fn snapshot(root: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                out.push((entry.path(), meta.len(), meta.modified().unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// A suggestion is advice, not evidence: proposing an edge leaves every answer
/// computed from the register exactly as it was.
#[test]
fn a_readonly_suggestion_is_not_evidence() {
    let case = Case::emitting(TWO_NODES);
    let bare = case.run(&["query", "counts", "--format", "plain"]);
    let path = write_doc(
        &case,
        &document(vec![suggestion("REQ-2", "REQ-1", 0.9, "b")]),
    );
    let with = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ]);
    let after = case.run(&["query", "counts", "--format", "plain"]);

    assert_eq!(with.status.code(), Some(0));
    assert_eq!(
        bare.stdout, after.stdout,
        "the register moved under a suggestion"
    );
}

// Requirement: Severity override in profile

/// The overlay's codes ship as hints, so a profile trying to promote one is a
/// CONFIG_ERROR and the suggestion still renders as advice.
#[test]
fn a_promoted_overlay_hint_is_refused_and_reported() {
    let profile =
        format!("{MINIMAL_PROFILE}validations:\n  - SUGGESTED_EDGE:\n      severity: warning\n");
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{TWO_NODES}\nDOC"));
    let path = write_doc(
        &case,
        &document(vec![suggestion("REQ-1", "REQ-2", 0.9, "b")]),
    );
    let output = case.run(&[
        "validate",
        "--format",
        "plain",
        "--suggestions",
        path.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    assert!(stdout.contains("CONFIG_ERROR"), "{stdout}");
    assert!(stdout.contains("SUGGESTED_EDGE"), "{stdout}");
    for line in stdout
        .lines()
        .filter(|l| l.contains("SUGGESTED_EDGE") && !l.contains("CONFIG_ERROR"))
    {
        assert!(line.starts_with("HINT"), "{line}");
    }
}
