//! The `cli` capability's scenarios, driven through the built binary.
//!
//! These run the real process rather than calling the library, because what they
//! check — the three-valued exit code, which stream a payload lands on, the
//! `--format` default — exists only there.
//!
//! Adapters are shell scripts written per case. An adapter is a program the core
//! runs and reads stdout from, so a script is a complete one.

mod common;

use common::{Case, MINIMAL_PROFILE, binary, code, stderr, stdout};

/// A well-formed document with one node that has no edges: one `ORPHAN_NODE`
/// warning, so the register is clean at the default severities.
const CLEAN_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}}]}"#;

/// The same register with a dangling edge, which is an error-severity finding.
const ERROR_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}}],
  "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
             "provenance": {"file": "r.md", "line": 1}}]}"#;

// Requirement: Exit codes

#[test]
fn a_clean_register_exits_zero() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("WARNING ORPHAN_NODE"));
}

#[test]
fn error_severity_findings_exit_one() {
    let case = Case::emitting(ERROR_DOCUMENT);
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("ERROR VACANCY"));
}

#[test]
fn strict_promotes_warnings_into_a_nonzero_exit() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    assert_eq!(code(&case.run(&["validate", "--format", "plain"])), 0);
    assert_eq!(
        code(&case.run(&["validate", "--strict", "--format", "plain"])),
        1
    );
    // trace takes --strict on the same terms; summary does not take it at all.
    assert_eq!(
        code(&case.run(&["trace", "--strict", "--format", "plain"])),
        1
    );
    let refused = case.run(&["summary", "--strict", "--format", "plain"]);
    assert_ne!(code(&refused), 0, "summary must not accept --strict");
}

#[test]
fn an_unreadable_profile_exits_two() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    std::fs::write(&case.profile, "name: t\nnode_kinds: {}\n").unwrap();
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2, "a broken setup is never exit 1");
    assert!(
        stderr(&output).contains("Profile error"),
        "{}",
        stderr(&output)
    );
}

// Requirement: Adapter loading

#[test]
fn an_adapter_that_cannot_be_run_exits_two_and_names_it() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    std::fs::remove_file(&case.adapter).unwrap();
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("could not run adapter"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("adapter"),
        "the message names the program"
    );
    assert!(
        stdout(&output).is_empty(),
        "no findings from a run that did not happen"
    );
}

#[test]
fn an_adapter_that_exits_nonzero_exits_two_and_carries_its_diagnosis() {
    let case = Case::running("echo 'could not read REQUIREMENTS.md' >&2; exit 3");
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("exited 3"), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("could not read REQUIREMENTS.md"),
        "the adapter's own diagnosis is usually why the run failed"
    );
}

#[test]
fn an_adapter_that_emits_nothing_exits_two_rather_than_reporting_an_empty_register() {
    let case = Case::running("exit 0");
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(
        code(&output),
        2,
        "silence must never read as 'nothing to see'"
    );
    assert!(
        stderr(&output).contains("emitted no output"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_off_schema_document_exits_two_and_names_the_failure() {
    let case = Case::emitting(r#"{"interface_version": "1.0", "nodes": [{"kind": "req"}]}"#);
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("missing 'id'"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_unsupported_interface_version_exits_two() {
    let case = Case::emitting(r#"{"interface_version": "9.9", "nodes": []}"#);
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("unsupported interface version"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn unparseable_adapter_output_exits_two() {
    let case = Case::emitting("not json at all");
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("could not parse adapter output as JSON"));
}

// Requirement: Summary subcommand

/// `MINIMAL_PROFILE` with a rollup over `req`, plus whatever extra YAML is given.
fn summary_profile(extra: &str) -> String {
    format!(
        "{MINIMAL_PROFILE}validations:\n  - SUMMARY:\n      node_kind: req\n\
         \x20     status_attr: status\n      group_by_attr: file\n{extra}"
    )
}

#[test]
fn summary_writes_its_rollup_to_stdout_and_adapter_issues_to_stderr() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req",
                 "attrs": {"status": "done", "file": "a.md"},
                 "provenance": {"file": "a.md", "line": 1}}],
      "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "bad row",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile(""),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let output = case.run(&["summary", "--format", "json"]);

    assert_eq!(code(&output), 0, "a warning is not an error");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("stdout stays a parseable rollup");
    assert_eq!(parsed["totals"]["total"], 1);
    // The issues honour the requested format on their own stream.
    assert!(
        stderr(&output).contains("\"code\": \"PARSE_ERROR\""),
        "{}",
        stderr(&output)
    );
}

#[test]
fn summary_exits_on_an_error_severity_adapter_issue() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile(""),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    assert_eq!(code(&case.run(&["summary", "--format", "plain"])), 1);
}

// Requirement: Summary rejects a mistyped config

#[test]
fn summary_config_wrong_type_is_exit_2_not_an_empty_rollup() {
    let profile = format!(
        "{MINIMAL_PROFILE}validations:\n  - SUMMARY:\n      node_kind: 123\n\
         \x20     status_attr: status\n      group_by_attr: file\n"
    );
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"));
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(
        code(&output),
        2,
        "a mistyped config is a broken setup, not a clean empty rollup"
    );
    let err = stderr(&output);
    assert!(
        err.contains("node_kind") && err.contains("int"),
        "stderr names the key and the type: {err}"
    );
}

// Requirement: CLI entry point

#[test]
fn summary_honours_a_profile_override_on_an_adapter_code() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile("  - PARSE_ERROR:\n      severity: info\n"),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(
        code(&output),
        0,
        "the profile already said that severity is wrong"
    );
    assert!(
        stderr(&output).contains("INFO PARSE_ERROR"),
        "{}",
        stderr(&output)
    );
}

// Requirement: Summary subcommand

#[test]
fn a_profile_with_no_summary_config_exits_two() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert_eq!(
        stderr(&output).trim(),
        "Error: profile has no SUMMARY validation config"
    );
}

#[test]
fn a_profile_with_two_summary_configs_exits_two_rather_than_choosing() {
    let case = Case::with_profile(
        &summary_profile(
            "  - SUMMARY:\n      node_kind: req\n      status_attr: status\n\
             \x20     group_by_attr: kind\n",
        ),
        &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("cannot choose between them"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_summary_config_missing_a_required_key_exits_two() {
    let case = Case::with_profile(
        &format!("{MINIMAL_PROFILE}validations:\n  - SUMMARY:\n      node_kind: req\n"),
        &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("requires node_kind, status_attr, group_by_attr"),
        "{}",
        stderr(&output)
    );
}

// Requirement: Format flag

#[test]
fn the_format_defaults_to_plain_when_stdout_is_not_a_terminal() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    // A captured pipe is never a terminal, so the default is observable here.
    let output = case.run(&["validate"]);
    assert_eq!(
        stdout(&output).trim(),
        "WARNING ORPHAN_NODE r.md:1 node 'REQ-1' has no edges",
        "rich would have padded the columns"
    );
}

#[test]
fn an_unknown_format_is_refused_by_the_parser() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let output = case.run(&["validate", "--format", "yaml"]);
    assert_ne!(code(&output), 0);
    assert!(stderr(&output).contains("yaml"), "{}", stderr(&output));
}

// Requirement: Version flag

#[test]
fn the_version_flag_prints_the_package_version_and_exits_zero() {
    let output = std::process::Command::new(binary())
        .arg("--version")
        .output()
        .expect("the lattice binary runs");

    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout(&output).trim(),
        format!("lattice, version {}", env!("CARGO_PKG_VERSION"))
    );
}

// Requirement: Trace report structure

#[test]
fn trace_renders_its_report_in_each_format() {
    let case = Case::emitting(CLEAN_DOCUMENT);

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&case.run(&["trace", "--format", "json"])))
            .expect("trace json is parseable");
    assert_eq!(parsed["entries"][0]["id"], "REQ-1");
    assert_eq!(parsed["header"]["profile"], "t");

    let plain = stdout(&case.run(&["trace", "--format", "plain"]));
    assert!(plain.contains("REQ-1"), "{plain}");
    assert!(plain.contains("edges:0 findings:1"), "{plain}");

    let rich = stdout(&case.run(&["trace", "--format", "rich"]));
    assert!(rich.contains("Key Attr"), "{rich}");
    assert!(rich.contains("1 entries, 1 warning(s)"), "{rich}");
}

// Requirement: Exit codes for trace

#[test]
fn trace_exits_one_on_an_error_severity_finding() {
    let case = Case::emitting(ERROR_DOCUMENT);
    let output = case.run(&["trace", "--format", "plain"]);
    assert_eq!(code(&output), 1);
    // The finding names REQ-1, which exists, so it hangs under that entry.
    assert!(
        stdout(&output).contains("ERROR VACANCY"),
        "{}",
        stdout(&output)
    );
}

// Requirement: Trace report structure

#[test]
fn trace_puts_a_finding_naming_no_declared_node_in_the_footer() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "r.md", "line": 1}}],
      "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "ghost",
                  "provenance": {"file": "r.md", "line": 9}, "node_id": "REQ-404"}]}"#;
    let case = Case::emitting(document);
    let out = stdout(&case.run(&["trace", "--format", "plain"]));

    assert!(out.contains("Unattachable findings:"), "{out}");
    assert!(
        !out.contains("REQ-404 "),
        "no entry is invented for it: {out}"
    );
}

// Requirement: Three output formats

#[test]
fn rich_output_carries_no_ansi_when_the_stream_is_not_a_terminal() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [],
      "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile(""),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let output = case.run(&["summary", "--format", "rich"]);

    // The issues go to stderr, which is captured here and so is not a terminal.
    assert!(
        stderr(&output).contains("PARSE_ERROR"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains('\u{1b}'),
        "an escape survived onto a captured stream"
    );
    assert!(!stdout(&output).contains('\u{1b}'));
}

// Requirement: Severity resolution is shared by every command

/// A profile binding `phase` to `PARSE_ERROR`, plus the rollup summary needs.
fn pathway_summary_profile() -> String {
    let base =
        summary_profile("  - PARSE_ERROR:\n      pathway: phase\n      position_attr: trigger\n");
    // `pathways:` is a top-level key, so it goes above the list rather than into it.
    base.replace("validations:", "pathways: [phase]\nvalidations:")
}

/// One node not yet due on the pathway, carrying one error-severity adapter issue.
const DEMOTABLE_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
  "nodes": [{"id": "REQ-1", "kind": "req",
             "attrs": {"status": "todo", "file": "a.md", "trigger": "M4"},
             "provenance": {"file": "a.md", "line": 1}}],
  "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
              "provenance": {"file": "a.md", "line": 9}, "node_id": "REQ-1"}]}"#;

#[test]
fn a_pathway_demotion_drops_the_exit_code_for_validate_and_summary_alike() {
    let case = Case::with_profile(
        &pathway_summary_profile(),
        &format!("cat <<'DOC'\n{DEMOTABLE_DOCUMENT}\nDOC"),
    );

    let validated = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&validated), 0, "{}", stdout(&validated));
    assert!(
        stdout(&validated).contains("INFO PARSE_ERROR"),
        "{}",
        stdout(&validated)
    );

    let summarised = case.run(&["summary", "--format", "plain"]);
    assert_eq!(
        code(&summarised),
        0,
        "summary agrees or the two have diverged"
    );
    assert!(
        stderr(&summarised).contains("INFO PARSE_ERROR"),
        "{}",
        stderr(&summarised)
    );
}

#[test]
fn the_same_issue_unbound_still_errors() {
    // The control for the case above: without the binding it is exit 1, so the
    // demotion is what changed the outcome rather than the document.
    let case = Case::with_profile(
        &summary_profile(""),
        &format!("cat <<'DOC'\n{DEMOTABLE_DOCUMENT}\nDOC"),
    );
    assert_eq!(code(&case.run(&["validate", "--format", "plain"])), 1);
    assert_eq!(code(&case.run(&["summary", "--format", "plain"])), 1);
}

// Requirement: Findings the pathway pass cannot resolve

#[test]
fn a_binding_the_register_cannot_satisfy_is_a_finding_and_never_exit_two() {
    // Same profile and a bound finding, but a document with no pathway at all. The
    // finding is what the pass tries to resolve, so without one there is nothing
    // to report the mismatch about.
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req",
                 "attrs": {"status": "todo", "file": "a.md"},
                 "provenance": {"file": "a.md", "line": 1}}],
      "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": "REQ-1"}]}"#;
    let case = Case::with_profile(
        &pathway_summary_profile(),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert_ne!(
        code(&output),
        2,
        "the profile loaded and the adapter honoured its contract"
    );
    assert!(
        stdout(&output).contains("WARNING PATHWAY_UNRESOLVED <profile>:0"),
        "{}",
        stdout(&output)
    );
}

// Requirement: Summary subcommand

#[test]
fn the_rollup_carries_a_column_for_every_declared_status() {
    let profile = "name: t\nprofile_version: \"1.0.0\"\nnode_kinds:\n  req:\n\
         \x20   id_pattern: \"^REQ-\\\\d+$\"\n    attrs:\n\
         \x20     status: {type: enum, values: [blocked, done, todo]}\n\
         edge_kinds: {}\nvalidations:\n  - SUMMARY:\n      node_kind: req\n\
         \x20     status_attr: status\n      group_by_attr: file\n";
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {"status": "done", "file": "a.md"},
                 "provenance": {"file": "a.md", "line": 1}}]}"#;
    let case = Case::with_profile(profile, &format!("cat <<'DOC'\n{document}\nDOC"));
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&case.run(&["summary", "--format", "json"]))).unwrap();

    // A declared status no node carries is a zero column, not an absent key.
    assert_eq!(parsed["files"][0]["blocked"], 0);
    assert_eq!(parsed["files"][0]["todo"], 0);
    assert_eq!(parsed["files"][0]["done"], 1);
    assert_eq!(parsed["totals"]["total"], 1);
}

// Requirement: Duplicate node IDs are resolved at ingest

#[test]
fn a_duplicate_in_the_document_is_a_finding_rather_than_exit_two() {
    // Ingest resolves it, so the core still has a view of the register.
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "r.md", "line": 1}},
                {"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "r.md", "line": 5}}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(
        code(&output),
        1,
        "an error-severity finding, not a broken setup"
    );
    assert!(
        stdout(&output).contains("ERROR PARSE_ERROR"),
        "{}",
        stdout(&output)
    );
}

// Requirement: Three output formats

#[test]
fn findings_json_carries_the_node_id() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&case.run(&["validate", "--format", "json"]))).unwrap();
    assert_eq!(parsed["findings"][0]["node_id"], "REQ-1");
}

// Requirement: Core invokes the adapter with profile and target paths

#[test]
fn the_adapter_receives_the_profile_and_target_it_was_given() {
    // The adapter writes its own arguments into the document it emits, so what
    // reaches it is observable in what comes back. The profile path is the
    // core's resolved document, not the user's file.
    let case = Case::running(
        r#"printf '{"interface_version": "1.0", "issues": [{"severity": "warning",
             "code": "ARGV", "message": "%s", "provenance": {"file": "r.md", "line": 1},
             "node_id": null}]}' "$*""#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    let seen = stdout(&output);
    assert!(seen.contains("--profile"), "{seen}");
    assert!(seen.contains("--target"), "{seen}");
    assert!(
        !seen.contains(case.profile.to_str().unwrap()),
        "the user's profile path must not reach the adapter: {seen}"
    );
}

#[test]
fn the_profile_path_names_the_resolved_document() {
    // The adapter counts `resolved_schema` occurrences in the file behind the
    // profile path it received ($2), so the handoff content is observable.
    let case = Case::running(
        r#"printf '{"interface_version": "1.0", "issues": [{"severity": "warning",
             "code": "SEEN", "message": "schema markers: %s", "provenance": {"file": "r.md", "line": 1},
             "node_id": null}]}' "$(grep -c resolved_schema "$2")""#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert!(
        stdout(&output).contains("schema markers: 1"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn an_adapters_stderr_never_becomes_its_document() {
    let case = Case::running(&format!(
        "echo 'a warning nobody asked for' >&2\ncat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"
    ));
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 0, "chatter on stderr is not a failure");
    assert!(!stdout(&output).contains("nobody asked for"));
}

// Requirement: Adapter failure is a broken setup, not a finding

#[test]
fn a_non_executable_adapter_exits_two() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let mut perms = std::fs::metadata(&case.adapter).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o644);
    std::fs::set_permissions(&case.adapter, perms).unwrap();

    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("could not run adapter"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_adapter_that_writes_a_document_then_fails_is_still_exit_two() {
    // Partial output plus a non-zero exit is a broken adapter, not a register
    // with findings — reading the document would be trusting an aborted run.
    let case = Case::running(&format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC\nexit 1"));
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
}

// Requirement: Hint severity tier

#[test]
fn hint_only_findings_exit_zero_with_and_without_strict() {
    let profile =
        format!("{MINIMAL_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: hint\n");
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"));

    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("HINT ORPHAN_NODE"));
    assert_eq!(
        code(&case.run(&["validate", "--strict", "--format", "plain"])),
        0,
        "a hint is never a reason for a strict run to fail"
    );
}

// Requirement: Resolve command

#[test]
fn resolve_prints_the_handoff_document_an_adapter_reads() {
    let case = Case::emitting("{}");
    let output = case_resolve(&case);

    assert_eq!(output.status.code(), Some(0));
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is the resolved JSON document");
    assert_eq!(parsed["resolved_schema"], "1");
    assert!(parsed["node_kinds"]["req"].is_object(), "{parsed}");
}

/// The point of the command: a program that consumes a register can be run by
/// hand on what it prints, without reimplementing profile resolution.
#[test]
fn an_adapter_runs_on_what_resolve_printed() {
    let case = Case::emitting("{}");
    let resolved = case.dir.join("resolved.json");
    std::fs::write(&resolved, case_resolve(&case).stdout).unwrap();

    let output = std::process::Command::new(&case.adapter)
        .arg("--profile")
        .arg(&resolved)
        .arg("--target")
        .arg(&case.dir)
        .output()
        .expect("the adapter program runs");

    assert!(output.status.success(), "{output:?}");
}

#[test]
fn resolve_on_an_unloadable_profile_is_exit_two() {
    let case = Case::with_profile("name: t\nnode_kinds: 3\n", "true");
    let output = case_resolve(&case);

    assert_eq!(output.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}

/// `resolve` takes neither `--adapter` nor `--target`, so it cannot use
/// `Case::run`, which supplies both.
fn case_resolve(case: &Case) -> std::process::Output {
    std::process::Command::new(binary())
        .arg("resolve")
        .arg("--profile")
        .arg(&case.profile)
        .output()
        .expect("the lattice binary runs")
}
