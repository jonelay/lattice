//! The `finding-notes` capability's scenarios: the notes file a profile names,
//! its entries, how they attach to findings, and when they expire.

mod common;

use std::path::{Path, PathBuf};

use common::{MINIMAL_PROFILE, scratch};
use latt_core::profile::{Profile, ProfileError, load_profile, load_profile_value};

/// Write each `(relative path, text)` under a fresh directory, creating parents.
fn tree(files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch();
    for (name, text) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn with_annotations(path: &str) -> String {
    format!("{MINIMAL_PROFILE}annotations: {path}\n")
}

fn load(dir: &Path, profile: &str) -> Result<Profile, ProfileError> {
    load_profile(&dir.join(profile))
}

// Requirement: graph-service/A finding's provenance admits additions

#[test]
fn an_entry_location_names_the_notes_file_and_the_entry_line() {
    let dir = tree(&[
        ("p.yaml", &with_annotations("notes.yaml")),
        (
            "notes.yaml",
            "- finding: {code: VACANCY, node: A}\n  note: first\n\
             - finding: {code: VACANCY, node: B}\n  note: second\n",
        ),
    ]);
    let profile = load(&dir, "p.yaml").unwrap();
    let at = profile
        .note_location("VACANCY", "B")
        .expect("B has an entry");
    assert!(at.file().ends_with("notes.yaml"), "{}", at.file());
    assert_eq!(at.line(), std::num::NonZeroU32::new(3));
    assert_eq!(profile.note_location("VACANCY", "C"), None);
}

#[test]
fn a_flow_style_entry_has_a_location_without_a_line() {
    let dir = tree(&[
        ("p.yaml", &with_annotations("notes.yaml")),
        (
            "notes.yaml",
            "[{finding: {code: VACANCY, node: A}, note: flow}]\n",
        ),
    ]);
    let profile = load(&dir, "p.yaml").unwrap();
    let at = profile
        .note_location("VACANCY", "A")
        .expect("A has an entry");
    assert!(at.file().ends_with("notes.yaml"), "{}", at.file());
    assert_eq!(at.line(), None);
}

// Requirement: finding-notes/Notes file location

#[test]
fn location_resolves_beside_the_declaring_profile() {
    let child = MINIMAL_PROFILE.replace("name: t\n", "name: t\nextends: ../profiles/base.yaml\n");
    let dir = tree(&[
        ("profiles/base.yaml", &with_annotations("notes.yaml")),
        ("profiles/notes.yaml", ""),
        ("repo/lattice.yaml", &child),
    ]);
    let profile = load(&dir, "repo/lattice.yaml").unwrap();
    assert_eq!(
        profile.annotations().unwrap().canonicalize().unwrap(),
        dir.join("profiles/notes.yaml").canonicalize().unwrap()
    );
}

#[test]
fn location_reads_the_parents_file_not_one_beside_the_child() {
    let child = MINIMAL_PROFILE.replace("name: t\n", "name: t\nextends: ../profiles/base.yaml\n");
    let dir = tree(&[
        ("profiles/base.yaml", &with_annotations("notes.yaml")),
        ("profiles/notes.yaml", "- finding: {code: VACANCY"),
        ("repo/lattice.yaml", &child),
        ("repo/notes.yaml", ""),
    ]);
    let error = load(&dir, "repo/lattice.yaml").unwrap_err().to_string();
    assert!(error.contains("profiles/notes.yaml"), "{error}");
}

#[test]
fn location_child_declaration_wins() {
    let child = format!(
        "{}annotations: b.yaml\n",
        MINIMAL_PROFILE.replace("name: t\n", "name: t\nextends: parent.yaml\n")
    );
    let dir = tree(&[
        ("parent.yaml", &with_annotations("a.yaml")),
        ("a.yaml", "not: [a list"),
        ("b.yaml", ""),
        ("child.yaml", &child),
    ]);
    let profile = load(&dir, "child.yaml").unwrap();
    assert!(profile.annotations().unwrap().ends_with("b.yaml"));
}

#[test]
fn location_missing_file_is_not_a_load_error() {
    let dir = tree(&[("p.yaml", &with_annotations("absent.yaml"))]);
    let profile =
        load(&dir, "p.yaml").expect("a missing notes file is a finding, not a load error");
    assert!(profile.annotations().unwrap().ends_with("absent.yaml"));
}

#[test]
fn location_unparseable_file_is_a_load_error_naming_it() {
    let dir = tree(&[
        ("p.yaml", &with_annotations("notes.yaml")),
        ("notes.yaml", "- finding: {code: VACANCY"),
    ]);
    let error = load(&dir, "p.yaml").unwrap_err().to_string();
    assert!(error.contains("notes.yaml"), "{error}");
}

#[test]
fn location_a_mapping_at_top_level_is_a_load_error() {
    let dir = tree(&[
        ("p.yaml", &with_annotations("notes.yaml")),
        ("notes.yaml", "finding: {code: VACANCY, node: REQ-1}\n"),
    ]);
    let error = load(&dir, "p.yaml").unwrap_err().to_string();
    assert!(
        error.contains("notes.yaml") && error.contains("list"),
        "{error}"
    );
}

#[test]
fn location_empty_file_holds_no_entries() {
    let dir = tree(&[
        ("p.yaml", &with_annotations("notes.yaml")),
        ("notes.yaml", ""),
    ]);
    assert!(load(&dir, "p.yaml").is_ok());
}

#[test]
fn location_non_string_value_is_a_load_error() {
    let dir = tree(&[("p.yaml", &with_annotations("[a, b]"))]);
    let error = load(&dir, "p.yaml").unwrap_err().to_string();
    assert!(error.contains("annotations"), "{error}");
}

#[test]
fn location_resolved_document_omits_annotations() {
    let dir = tree(&[
        ("p.yaml", &with_annotations("notes.yaml")),
        ("notes.yaml", ""),
    ]);
    let profile = load(&dir, "p.yaml").unwrap();
    let document: serde_json::Value =
        serde_json::from_str(&latt_core::profile::resolved_document(&profile).unwrap()).unwrap();
    assert!(document.get("annotations").is_none(), "{document}");
}

#[test]
fn location_a_value_loaded_profile_rejects_annotations() {
    let raw = serde_norway::from_str(&with_annotations("notes.yaml")).unwrap();
    let error = load_profile_value(raw, "host").unwrap_err().to_string();
    assert!(error.contains("annotations"), "{error}");
}

#[test]
fn location_unknown_key_error_lists_annotations() {
    let dir = tree(&[("p.yaml", &format!("{MINIMAL_PROFILE}validation: []\n"))]);
    let error = load(&dir, "p.yaml").unwrap_err().to_string();
    assert!(error.contains("\"annotations\""), "{error}");
}

// The graph the attach, expiry and code tests read: `REQ-1` carries one
// `VACANCY` (its edge names undeclared `REQ-9`) and `REQ-2` is unreferenced;
// pathway `stage` stands at `M2`.

use latt_core::graph::LatticeGraph;
use latt_core::types::{CivilDate, Issue, Severity, Until};
use latt_core::validate::validate_on;
use serde_json::json;

fn graph() -> LatticeGraph {
    common::ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "stage", "order": ["M1", "M2", "M3"], "current": "M2"}],
        "nodes": [
            {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "REQS.md", "line": 1}},
            {"id": "REQ-2", "kind": "req", "attrs": {}, "provenance": {"file": "REQS.md", "line": 2}},
        ],
        "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }))
}

/// A profile with `pathways: [stage]` and the given notes file text and
/// extra top-level YAML.
fn noted(notes: &str, extra: &str) -> Profile {
    let profile = format!("{MINIMAL_PROFILE}pathways: [stage]\nannotations: notes.yaml\n{extra}");
    let dir = tree(&[("p.yaml", &profile), ("notes.yaml", notes)]);
    load(&dir, "p.yaml").unwrap()
}

fn date(text: &str) -> CivilDate {
    CivilDate::parse(text).unwrap()
}

fn run(notes: &str) -> Vec<Issue> {
    validate_on(&graph(), &noted(notes, ""), false, date("2026-10-01"))
}

fn with_code<'a>(issues: &'a [Issue], code: &str) -> Vec<&'a Issue> {
    issues.iter().filter(|i| i.code == code).collect()
}

fn only<'a>(issues: &'a [Issue], code: &str) -> &'a Issue {
    let found = with_code(issues, code);
    assert_eq!(found.len(), 1, "expected one {code}: {issues:#?}");
    found[0]
}

const VACANCY_NOTE: &str = "- finding: {code: VACANCY, node: REQ-1}\n  note: Lands with M2.\n";

// Requirement: finding-notes/Note entry schema

#[test]
fn schema_a_complete_entry_loads() {
    let issues = run(
        "- finding: {code: VACANCY, node: REQ-1}\n  until: 2026-12-31\n  by: jone\n  at: 2026-10-01\n  note: Lands with M2.\n",
    );
    assert!(with_code(&issues, "NOTE_INVALID").is_empty(), "{issues:#?}");
    let note = only(&issues, "VACANCY").note.as_ref().unwrap();
    assert_eq!(note.text, "Lands with M2.");
    assert_eq!(note.until, Some(Until::Date(date("2026-12-31"))));
    assert_eq!(note.by.as_deref(), Some("jone"));
    assert_eq!(note.at, Some(date("2026-10-01")));
    assert!(!note.expired);
}

#[test]
fn schema_missing_note_text_is_invalid_at_its_line() {
    let notes =
        format!("{VACANCY_NOTE}# a comment\n\n\n- finding: {{code: UNREFERENCED, node: REQ-2}}\n");
    let issues = run(&notes);
    let invalid = only(&issues, "NOTE_INVALID");
    assert!(invalid.message.contains("note"), "{}", invalid.message);
    assert_eq!(
        invalid
            .origin
            .location()
            .expect("a file origin")
            .line()
            .map(u32::from),
        Some(6)
    );
    assert!(
        invalid
            .origin
            .location()
            .expect("a file origin")
            .file()
            .ends_with("notes.yaml")
    );
    assert_eq!(invalid.node_id, None);
    assert_eq!(invalid.severity, Severity::Error);
    assert!(
        with_code(&issues, "UNREFERENCED")
            .iter()
            .all(|i| i.note.is_none())
    );
}

#[test]
fn schema_unknown_key_is_named_with_the_accepted_keys() {
    let issues = run(&format!("{VACANCY_NOTE}  status: accepted\n"));
    let message = &only(&issues, "NOTE_INVALID").message;
    assert!(
        message.contains("status") && message.contains("until"),
        "{message}"
    );
    assert!(only(&issues, "VACANCY").note.is_none());
}

#[test]
fn schema_impossible_date_is_invalid() {
    let issues = run(&format!("{VACANCY_NOTE}  until: 2026-02-30\n"));
    assert!(only(&issues, "NOTE_INVALID").message.contains("2026-02-30"));
}

#[test]
fn schema_wrong_types_are_invalid() {
    for entry in [
        "- finding: {code: VACANCY}\n  note: x\n",
        "- finding: VACANCY\n  note: x\n",
        "- finding: {code: VACANCY, node: REQ-1}\n  note: ''\n",
        "- finding: {code: VACANCY, node: REQ-1}\n  note: x\n  by: [a]\n",
        "- finding: {code: VACANCY, node: REQ-1}\n  note: x\n  at: yesterday\n",
        "- finding: {code: VACANCY, node: REQ-1}\n  note: x\n  until: {pathway: phase, position: M2}\n",
        "- finding: {code: VACANCY, node: REQ-1, kind: req}\n  note: x\n",
        "- just a string\n",
    ] {
        let issues = run(entry);
        assert_eq!(
            with_code(&issues, "NOTE_INVALID").len(),
            1,
            "{entry}: {issues:#?}"
        );
        assert!(only(&issues, "VACANCY").note.is_none(), "{entry}");
    }
}

#[test]
fn schema_duplicate_entry_names_the_first_and_the_first_applies() {
    let notes = format!(
        "{VACANCY_NOTE}# gap\n# gap\n# gap\n- finding: {{code: VACANCY, node: REQ-1}}\n  note: Second.\n"
    );
    let issues = run(&notes);
    let invalid = only(&issues, "NOTE_INVALID");
    assert_eq!(
        invalid
            .origin
            .location()
            .expect("a file origin")
            .line()
            .map(u32::from),
        Some(6)
    );
    assert!(invalid.message.contains("line 1"), "{}", invalid.message);
    assert_eq!(
        only(&issues, "VACANCY").note.as_ref().unwrap().text,
        "Lands with M2."
    );
}

#[test]
fn schema_flow_style_items_carry_no_line() {
    let issues = run("[{finding: {code: VACANCY, node: REQ-9}, note: x}]\n");
    assert_eq!(
        only(&issues, "NOTE_UNUSED")
            .origin
            .location()
            .expect("a file origin")
            .line(),
        None
    );
}

#[test]
fn schema_a_flow_style_duplicate_names_no_line() {
    let issues = run("[{finding: {code: VACANCY, node: REQ-1}, note: a}, \
         {finding: {code: VACANCY, node: REQ-1}, note: b}]\n");
    let invalid = only(&issues, "NOTE_INVALID");
    assert!(
        invalid.message.ends_with("on 'REQ-1'"),
        "{}",
        invalid.message
    );
}

#[test]
fn schema_a_multiline_note_keeps_item_lines() {
    let notes = "- finding: {code: VACANCY, node: REQ-1}\n  note: |\n    first\n    second\n- finding: {code: VACANCY, node: REQ-7}\n  note: x\n";
    let issues = run(notes);
    assert_eq!(
        only(&issues, "NOTE_UNUSED")
            .origin
            .location()
            .expect("a file origin")
            .line()
            .map(u32::from),
        Some(5)
    );
}

// Requirement: finding-notes/Notes attach to findings

#[test]
fn attach_a_note_on_a_dangling_reference() {
    let issues = run(VACANCY_NOTE);
    let vacancy = only(&issues, "VACANCY");
    assert_eq!(vacancy.note.as_ref().unwrap().text, "Lands with M2.");
    assert_eq!(vacancy.severity, Severity::Error);
    assert!(vacancy.gates());
}

#[test]
fn attach_one_entry_to_every_matching_finding() {
    let graph = common::ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "REQS.md", "line": 1}}],
        "edges": [
            {"src": "REQ-1", "tgt": "REQ-8", "kind": "derives", "provenance": {"file": "REQS.md", "line": 1}},
            {"src": "REQ-1", "tgt": "REQ-9", "kind": "derives", "provenance": {"file": "REQS.md", "line": 1}},
        ],
    }));
    let issues = validate_on(&graph, &noted(VACANCY_NOTE, ""), false, date("2026-10-01"));
    let vacancies = with_code(&issues, "VACANCY");
    assert_eq!(vacancies.len(), 2);
    assert!(
        vacancies
            .iter()
            .all(|i| i.note.as_ref().is_some_and(|n| n.text == "Lands with M2."))
    );
    assert!(with_code(&issues, "NOTE_UNUSED").is_empty());
}

#[test]
fn attach_to_a_suppressed_finding() {
    let profile = noted(
        VACANCY_NOTE,
        "validations:\n  - SUPPRESS: {code: VACANCY, node_ids: [REQ-1]}\n",
    );
    let issues = validate_on(&graph(), &profile, false, date("2026-10-01"));
    let vacancy = only(&issues, "VACANCY");
    assert!(vacancy.suppressed);
    assert!(vacancy.note.is_some());
    assert!(with_code(&issues, "NOTE_UNUSED").is_empty());
}

#[test]
fn attach_a_stale_entry_is_unused() {
    let issues = run("- finding: {code: VACANCY, node: REQ-2}\n  note: x\n");
    let unused = only(&issues, "NOTE_UNUSED");
    assert!(unused.message.contains("VACANCY") && unused.message.contains("REQ-2"));
    assert_eq!(unused.severity, Severity::Info);
}

#[test]
fn attach_findings_without_a_note_carry_none() {
    let issues = run(VACANCY_NOTE);
    assert!(
        with_code(&issues, "UNREFERENCED")
            .iter()
            .all(|i| i.note.is_none())
    );
}

// Requirement: finding-notes/Note expiry

fn expiry(until: &str) -> Vec<Issue> {
    run(&format!("{VACANCY_NOTE}  until: {until}\n"))
}

#[test]
fn expiry_date_passed() {
    let issues = expiry("2026-09-30");
    let expired = only(&issues, "NOTE_EXPIRED");
    assert!(expired.message.contains("2026-09-30") && expired.message.contains("REQ-1"));
    assert_eq!(expired.severity, Severity::Warning);
    assert!(only(&issues, "VACANCY").note.as_ref().unwrap().expired);
}

#[test]
fn expiry_on_the_date_is_not_expired() {
    let issues = expiry("2026-10-01");
    assert!(with_code(&issues, "NOTE_EXPIRED").is_empty());
    assert!(!only(&issues, "VACANCY").note.as_ref().unwrap().expired);
}

#[test]
fn expiry_pathway_position_reached() {
    for position in ["M1", "M2"] {
        let issues = expiry(&format!("{{pathway: stage, position: {position}}}"));
        only(&issues, "NOTE_EXPIRED");
    }
}

#[test]
fn expiry_pathway_position_ahead() {
    let issues = expiry("{pathway: stage, position: M3}");
    assert!(with_code(&issues, "NOTE_EXPIRED").is_empty());
    assert_eq!(
        only(&issues, "VACANCY").note.as_ref().unwrap().until,
        Some(Until::Pathway {
            pathway: "stage".into(),
            position: "M3".into()
        })
    );
}

#[test]
fn expiry_position_not_on_the_pathway_is_invalid() {
    let issues = expiry("{pathway: stage, position: M9}");
    let message = &only(&issues, "NOTE_INVALID").message;
    assert!(
        message.contains("stage") && message.contains("M9"),
        "{message}"
    );
}

#[test]
fn expiry_a_pathway_the_register_does_not_supply_is_invalid() {
    let graph = common::ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "REQS.md", "line": 1}}],
        "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }));
    let profile = noted(
        &format!("{VACANCY_NOTE}  until: {{pathway: stage, position: M2}}\n"),
        "",
    );
    let issues = validate_on(&graph, &profile, false, date("2026-10-01"));
    assert!(only(&issues, "NOTE_INVALID").message.contains("stage"));
}

#[test]
fn expiry_an_expired_unused_entry_is_only_unused() {
    let issues = run("- finding: {code: VACANCY, node: REQ-2}\n  note: x\n  until: 2000-01-01\n");
    only(&issues, "NOTE_UNUSED");
    assert!(with_code(&issues, "NOTE_EXPIRED").is_empty());
}

// Requirement: finding-notes/Note finding codes

#[test]
fn codes_missing_file_is_a_warning_naming_the_path() {
    let profile = format!("{MINIMAL_PROFILE}annotations: absent.yaml\n");
    let dir = tree(&[("p.yaml", &profile)]);
    let profile = load(&dir, "p.yaml").unwrap();
    let issues = validate_on(&graph(), &profile, false, date("2026-10-01"));
    let missing = only(&issues, "NOTE_FILE_MISSING");
    assert_eq!(missing.severity, Severity::Warning);
    assert!(
        missing.message.contains("absent.yaml"),
        "{}",
        missing.message
    );
    assert_eq!(missing.node_id, None);
}

#[test]
fn codes_severity_override() {
    let profile = noted(
        &format!("{VACANCY_NOTE}  until: 2000-01-01\n"),
        "validations:\n  - NOTE_EXPIRED: {severity: error}\n",
    );
    let issues = validate_on(&graph(), &profile, false, date("2026-10-01"));
    assert_eq!(only(&issues, "NOTE_EXPIRED").severity, Severity::Error);
    assert!(
        only(&issues, "NOTE_EXPIRED").gates(),
        "an error finding sets exit 1"
    );
}

#[test]
fn codes_strict_promotes_a_missing_file() {
    let dir = tree(&[(
        "p.yaml",
        &format!("{MINIMAL_PROFILE}annotations: absent.yaml\n"),
    )]);
    let profile = load(&dir, "p.yaml").unwrap();
    let issues = validate_on(&graph(), &profile, true, date("2026-10-01"));
    assert_eq!(only(&issues, "NOTE_FILE_MISSING").severity, Severity::Error);
    assert!(
        only(&issues, "NOTE_FILE_MISSING").gates(),
        "an error finding sets exit 1"
    );
}

#[test]
fn codes_a_note_code_can_be_suppressed() {
    let profile = noted(
        "- finding: {code: VACANCY, node: REQ-2}\n  note: x\n",
        "validations:\n  - SUPPRESS: {code: NOTE_UNUSED}\n",
    );
    let issues = validate_on(&graph(), &profile, false, date("2026-10-01"));
    assert!(only(&issues, "NOTE_UNUSED").suppressed);
    assert!(with_code(&issues, "SUPPRESS_UNUSED").is_empty());
}

#[test]
fn codes_stale_notes_do_not_gate() {
    let dir = tree(&[
        (
            "p.yaml",
            &format!(
                "{}annotations: notes.yaml\n",
                MINIMAL_PROFILE.replace(
                    "allowed: [[req, req]]",
                    "allowed: [[req, req]]\nvalidations: []"
                )
            ),
        ),
        (
            "notes.yaml",
            "- finding: {code: VACANCY, node: REQ-5}\n  note: x\n",
        ),
    ]);
    let profile = load(&dir, "p.yaml").unwrap();
    let graph = common::ingest(json!({"interface_version": "1.0", "nodes": []}));
    let issues = validate_on(&graph, &profile, false, date("2026-10-01"));
    assert_eq!(
        issues
            .iter()
            .map(|i| i.code.as_str())
            .filter(|code| *code != "EMPTY_KIND")
            .collect::<Vec<_>>(),
        ["NOTE_UNUSED"]
    );
    assert!(!issues.iter().any(Issue::gates));
}

/// The four codes over the TOML fixture register, through the CLI, with a
/// profile that extends the shipped one and leaves it untouched. Needs the
/// native TOML adapter the shipped profile names.
#[cfg(feature = "native-adapters")]
#[test]
fn codes_over_the_toml_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let toml_profile = std::path::absolute(root.join("profiles/toml.yaml")).unwrap();
    let profile = format!(
        "name: toml-notes\nprofile_version: \"1.0.0\"\nextends: {}\nannotations: notes.yaml\n",
        toml_profile.display()
    );
    let notes = "\
- finding: {code: VACANCY, node: dangling/overdue}
  note: Target lands later.
- finding: {code: VACANCY, node: dangling/points_nowhere}
  note: Expired long ago.
  until: 2000-01-01
- finding: {code: VACANCY, node: alpha/two}
  note: Stale.
- finding: {code: VACANCY}
  note: Broken.
";
    let dir = tree(&[("lattice.yaml", &profile), ("notes.yaml", notes)]);
    let output = common::lattice()
        .args(["validate", "--format", "json", "--profile"])
        .arg(dir.join("lattice.yaml"))
        .arg("--target")
        .arg(root.join("tests/fixtures/mini-toml"))
        .output()
        .unwrap();
    assert_eq!(common::code(&output), 1, "{}", common::stderr(&output));
    let report: serde_json::Value = serde_json::from_str(&common::stdout(&output)).unwrap();
    let findings = report["findings"].as_array().unwrap();
    let count = |code: &str| findings.iter().filter(|f| f["code"] == code).count();
    assert_eq!(
        [
            count("NOTE_INVALID"),
            count("NOTE_UNUSED"),
            count("NOTE_EXPIRED")
        ],
        [1, 1, 1]
    );
    let noted = |node: &str| {
        findings
            .iter()
            .find(|f| f["code"] == "VACANCY" && f["node_id"] == node)
            .unwrap()["note"]
            .clone()
    };
    assert_eq!(
        noted("dangling/overdue"),
        json!({"text": "Target lands later."})
    );
    assert_eq!(noted("dangling/points_nowhere")["expired"], true);

    std::fs::remove_file(dir.join("notes.yaml")).unwrap();
    let output = common::lattice()
        .args(["validate", "--format", "json", "--profile"])
        .arg(dir.join("lattice.yaml"))
        .arg("--target")
        .arg(root.join("tests/fixtures/mini-toml"))
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_str(&common::stdout(&output)).unwrap();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "NOTE_FILE_MISSING")
    );
}
