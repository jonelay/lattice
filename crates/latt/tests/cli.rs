//! The `cli` capability's scenarios, driven through the built binary.
//!
//! These run the real process rather than calling the library, because what they
//! check (the three-valued exit code, which stream a payload lands on, the
//! `--format` default) exists only there.
//!
//! Adapters are shell scripts written per case. An adapter is a program the core
//! runs and reads stdout from, so a script is a complete one.

mod common;

use common::{Case, MINIMAL_PROFILE, code, lattice, stderr, stdout};

/// A well-formed document with one node that has no edges: `UNREFERENCED` and
/// `UNTRACED` warnings, so the register is clean at the default severities.
const CLEAN_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}}]}"#;

/// The same register with a dangling edge, which is an error-severity finding.
const ERROR_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}}],
  "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
             "provenance": {"file": "r.md", "line": 1}}]}"#;

// Requirement: cli/Exit codes

#[test]
fn a_clean_register_exits_zero() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("WARNING UNREFERENCED"));
}

// A report that could not be delivered is not a result: a full disk must not read
// as exit 0 over a truncated file, nor as a panic's 101.
#[cfg(target_os = "linux")]
#[test]
fn an_unwritable_stdout_exits_two() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let full = std::fs::File::create("/dev/full").expect("/dev/full opens");
    let output = case
        .command(&["validate", "--format", "json"])
        .stdout(full)
        .output()
        .expect("the lattice binary runs");
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("could not write output"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("panicked"), "{}", stderr(&output));
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

// Requirement: cli/Format flag

#[test]
fn an_unknown_format_is_a_usage_error_at_exit_two() {
    // clap's usage-error exit and the config exit coincide, so a broken
    // invocation reads the same as a broken setup to a caller.
    let case = Case::emitting(CLEAN_DOCUMENT);
    let output = case.run(&["validate", "--format", "yaml"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stderr(&output).contains("yaml"), "{}", stderr(&output));
    assert!(stderr(&output).contains("plain"), "{}", stderr(&output));
}

/// The `--format` help line and clap's errors for a bad value, as the
/// `ValueEnum` derive produced them before `Format` dropped it. The padding
/// follows the command's longest option, so adding a longer one widens it.
const PINNED_FORMAT_HELP: &str = "      --format <FORMAT>            Output format (default: auto-detect) \
     [possible values: plain, json, rich]";
const PINNED_FORMAT_XML: &str = "error: invalid value 'xml' for '--format <FORMAT>'\n  \
     [possible values: plain, json, rich]\n\nFor more information, try '--help'.\n";
const PINNED_FORMAT_JSN: &str = "error: invalid value 'jsn' for '--format <FORMAT>'\n  \
     [possible values: plain, json, rich]\n\n  tip: a similar value exists: 'json'\n\n\
     For more information, try '--help'.\n";

// Requirement: graph-service/Supported service shapes admit additions
// `Format`'s parser is the library's `FromStr`; the CLI's bytes are not.
#[test]
fn format_flag_keeps_its_help_and_error_bytes() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let help = case.run(&["validate", "--help"]);
    assert_eq!(code(&help), 0, "{}", stderr(&help));
    let help = stdout(&help);
    let line = help.lines().find(|l| l.contains("--format")).unwrap();
    assert_eq!(line, PINNED_FORMAT_HELP);
    for (value, pinned) in [("xml", PINNED_FORMAT_XML), ("jsn", PINNED_FORMAT_JSN)] {
        let output = case.run(&["validate", "--format", value]);
        assert_eq!(code(&output), 2);
        assert_eq!(stderr(&output), pinned);
    }
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

// Requirement: cli/Adapter loading

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

#[test]
fn adapter_output_that_is_not_utf8_exits_two_and_names_the_adapter() {
    // `\351` is a lone Latin-1 `é`: invalid UTF-8, so a lossy decode would ingest
    // `REQ-\u{FFFD}` as though the adapter had written it.
    let case = Case::running(
        r#"printf '{"interface_version": "1.2", "nodes": [{"id": "REQ-\351", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 1}}]}'"#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let text = stderr(&output);
    assert!(text.contains("output is not UTF-8"), "{text}");
    assert!(text.contains(&case.adapter.display().to_string()), "{text}");
}

#[test]
fn closed_keys_unknown_top_level_key_exits_two_and_names_it() {
    let case = Case::emitting(r#"{"interface_version": "1.2", "node": []}"#);
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("document: unknown key 'node'"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn closed_keys_node_with_attr_for_attrs_exits_two() {
    let case = Case::emitting(
        r#"{"interface_version": "1.2", "nodes": [{"id": "REQ-1", "kind": "req", "attr": {},
            "provenance": {"file": "r.md", "line": 1}}]}"#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("node 0: unknown key 'attr'"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn alias_error_finding_under_issues_beside_empty_findings_gates() {
    // Reading `findings` because the key exists would drop this error and exit 0.
    let case = Case::emitting(
        r#"{"interface_version": "1.2", "findings": [], "nodes": [],
            "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                        "provenance": {"file": "r.md", "line": 1}, "node_id": null}]}"#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("PARSE_ERROR"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn alias_arrays_both_populated_exit_two() {
    let finding = r#"{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                      "provenance": {"file": "r.md", "line": 1}, "node_id": null}"#;
    let case = Case::emitting(&format!(
        r#"{{"contract_version": "1.2", "findings": [{finding}], "issues": [{finding}]}}"#
    ));
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("'findings' and 'issues' are both non-empty"),
        "{}",
        stderr(&output)
    );
}

// `coverage` and `query` share the load `validate` runs; pin that they share its
// exit 2, so a refactor of that path cannot quietly turn a broken setup into 1.
#[test]
fn every_loading_command_exits_two_when_the_adapter_cannot_answer() {
    let failing = Case::running("echo 'register unreadable' >&2; exit 3");
    let garbled = Case::emitting("not json at all");
    for args in [
        &["coverage", "--format", "plain"][..],
        &["query", "counts", "--format", "plain"][..],
        &["trace", "--format", "plain"][..],
    ] {
        let output = failing.run(args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
        assert!(
            stderr(&output).contains("exited 3"),
            "{args:?}: {}",
            stderr(&output)
        );
        let output = garbled.run(args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
        assert!(
            stderr(&output).contains("could not parse adapter output as JSON"),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

#[cfg(unix)]
#[test]
fn an_adapter_killed_by_a_signal_exits_two_and_says_so() {
    let case = Case::running("kill -9 $$");
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("exited by a signal"),
        "{}",
        stderr(&output)
    );
}

// The adapter's stderr is carried into the message, capped from the front so the
// last words — usually the reason — survive, and cut on a character boundary.
#[test]
fn a_long_adapter_diagnosis_keeps_its_tail_within_the_cap() {
    let case = Case::running(
        "i=0; while [ $i -lt 700 ]; do printf 'é—x' >&2; i=$((i+1)); done; \
         echo 'THE REASON' >&2; exit 1",
    );
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let text = stderr(&output);
    assert!(text.contains("…"), "the cut is marked: {text}");
    assert!(text.trim_end().ends_with("THE REASON"), "{text}");
    let detail = text.trim_end().split_once('…').expect("marker present").1;
    assert!(detail.chars().count() <= 2000, "{}", detail.chars().count());
    assert!(!text.contains("panicked"), "{text}");
}

// Requirement: coverage-query/Summary subcommand

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

// Requirement: cli/Exit codes
#[test]
fn summary_does_not_exit_on_an_adapter_issue_the_profile_suppressed() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req",
                 "attrs": {"status": "done", "file": "a.md"},
                 "provenance": {"file": "a.md", "line": 1}}],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile("  - SUPPRESS:\n      code: PARSE_ERROR\n"),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let plain = case.run(&["summary", "--format", "plain"]);
    assert_eq!(
        code(&plain),
        0,
        "validate exits 0 on the same profile: {}",
        stderr(&plain)
    );
    assert!(
        !stderr(&plain).contains("PARSE_ERROR"),
        "plain omits a suppressed finding: {}",
        stderr(&plain)
    );
    let json = case.run(&["summary", "--format", "json"]);
    assert_eq!(code(&json), 0);
    assert!(
        stderr(&json).contains("\"suppressed\": true"),
        "json keeps the record and marks it: {}",
        stderr(&json)
    );
}

#[test]
fn a_query_omits_an_adapter_issue_the_profile_suppressed_from_plain_stderr() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                 "provenance": {"file": "a.md", "line": 1}}],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &format!("{MINIMAL_PROFILE}validations:\n  - SUPPRESS:\n      code: PARSE_ERROR\n"),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let output = case.run(&["query", "counts", "--format", "plain"]);
    assert_eq!(code(&output), 0);
    assert!(
        !stderr(&output).contains("PARSE_ERROR"),
        "{}",
        stderr(&output)
    );
}

// When every adapter issue is suppressed there is nothing to report: a blank
// line or "No findings." on stderr reads as a diagnostic the user must inspect.
// Requirement: cli/CLI entry point
#[test]
fn stderr_is_empty_when_every_adapter_issue_is_suppressed() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req",
                 "attrs": {"status": "done", "file": "a.md"},
                 "provenance": {"file": "a.md", "line": 1}}],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile("  - SUPPRESS:\n      code: PARSE_ERROR\n"),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    for args in [&["summary"][..], &["query", "counts"][..]] {
        for format in ["plain", "rich"] {
            let output = case.run(&[args, &["--format", format][..]].concat());
            assert_eq!(code(&output), 0, "{args:?} {format}: {}", stderr(&output));
            assert_eq!(stderr(&output), "", "{args:?} {format}");
        }
    }
}

// Requirement: cli/Summary rejects a mistyped config

// A status literally named `total` would share the synthetic column's key, and
// be counted twice there.
#[test]
fn a_status_named_total_exits_two_rather_than_double_counting() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [{"id": "REQ-1", "kind": "req",
                 "attrs": {"status": "total", "file": "a.md"},
                 "provenance": {"file": "a.md", "line": 1}}]}"#;
    let case = Case::with_profile(
        &summary_profile(""),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let output = case.run(&["summary", "--format", "json"]);
    assert_eq!(code(&output), 2, "{}", stdout(&output));
    assert!(stderr(&output).contains("'total'"), "{}", stderr(&output));
}

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

// Requirement: cli/CLI entry point

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

// Requirement: coverage-query/Summary subcommand

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

// Requirement: cli/Format flag

#[test]
fn the_format_defaults_to_plain_when_stdout_is_not_a_terminal() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    // A captured pipe is never a terminal, so the default is observable here.
    let output = case.run(&["validate"]);
    assert_eq!(
        stdout(&output).trim(),
        "INFO EMPTY_KIND <profile>:0 edge kind 'derives' has no edge\n\
         WARNING UNREFERENCED r.md:1 node 'REQ-1' has no incoming edges\n\
         WARNING UNTRACED r.md:1 node 'REQ-1' has no outgoing edges",
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

// Requirement: cli/Version flag

#[test]
fn the_version_flag_prints_the_package_version_and_exits_zero() {
    let output = common::lattice()
        .arg("--version")
        .output()
        .expect("the lattice binary runs");

    assert_eq!(code(&output), 0);
    assert_eq!(
        stdout(&output).trim(),
        format!("latt, version {}", env!("CARGO_PKG_VERSION"))
    );
}

// Requirement: cli/CLI entry point

// `cargo install latt` builds browse; an embedder of the library never compiles clap or
// the terminal crates, whatever features it picks.
#[test]
fn the_tui_is_a_default_of_the_binary_and_absent_from_the_library() {
    let read = |path: &str| {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
            .expect("the package manifest is readable")
    };
    let binary = read("Cargo.toml");
    let default = binary
        .lines()
        .find(|line| line.starts_with("default = "))
        .expect("the manifest declares default features");
    assert!(default.contains("\"tui\""), "{default}");

    // Every section but `[dev-dependencies]`: the benchmark example's clap is a dev edge.
    let library = read("../latt-core/Cargo.toml");
    let mut section = "";
    for line in library.lines() {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if section == "[dev-dependencies]" {
            continue;
        }
        for name in [
            "tui",
            "clap",
            "ratatui",
            "tui-markdown",
            "signal-hook",
            "crossterm",
        ] {
            let declared =
                line.starts_with(&format!("{name} ")) || line.starts_with(&format!("{name}."));
            assert!(!declared, "latt-core's {section} declares {name}");
        }
    }
}

#[cfg(not(feature = "tui"))]
#[test]
fn browse_is_an_unrecognized_subcommand_without_the_tui_feature() {
    let output = common::lattice()
        .arg("browse")
        .output()
        .expect("the lattice binary runs");

    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("unrecognized subcommand"),
        "{}",
        stderr(&output)
    );
}

#[cfg(feature = "tui")]
#[test]
fn browse_is_a_subcommand_with_the_tui_feature() {
    let output = common::lattice()
        .args(["browse", "--help"])
        .output()
        .expect("the lattice binary runs");

    assert_eq!(code(&output), 0, "{}", stderr(&output));
}

#[cfg(feature = "tui")]
#[test]
fn browse_without_a_terminal_exits_2_before_reading_the_profile() {
    // Requirement: browse/Exit contract
    // The test captures stdout, so browse has no terminal to draw on; the cause, not
    // an OS error, is the message.
    let output = common::lattice()
        .env("TERM", "xterm-256color")
        .args([
            "browse",
            "--profile",
            "/nonexistent/profile.yaml",
            "--target",
            ".",
        ])
        .output()
        .expect("the lattice binary runs");

    let err = stderr(&output);
    assert_eq!(code(&output), 2, "{err}");
    assert!(err.contains("needs a terminal"), "{err}");
    assert!(err.contains("--format plain"), "{err}");
    assert!(
        !err.contains("profile.yaml"),
        "the profile was read first: {err}"
    );
    assert!(output.stdout.is_empty());
}

#[cfg(feature = "tui")]
#[test]
fn browse_dumb_terminal_exits_2_before_reading_the_profile() {
    // Requirement: browse/Exit contract — TERM=dumb. The profile does not exist, so a
    // message about it would mean the check ran too late.
    let output = common::lattice()
        .env("TERM", "dumb")
        .args([
            "browse",
            "--profile",
            "/nonexistent/profile.yaml",
            "--target",
            ".",
        ])
        .output()
        .expect("the lattice binary runs");

    let err = stderr(&output);
    assert_eq!(code(&output), 2, "{err}");
    assert!(err.contains("cursor addressing"), "{err}");
    assert!(err.contains("--format plain"), "{err}");
    assert!(
        !err.contains("profile.yaml"),
        "the profile was read first: {err}"
    );
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
}

// Requirement: browse/Terminal lifecycle

/// Browse running in a pty that util-linux `script` supplies. `script -e`
/// exits with browse's code, and its stdout is everything browse drew.
#[cfg(all(feature = "tui", target_os = "linux"))]
struct PtyBrowse {
    script: std::process::Child,
    output: std::process::ChildStdout,
    pid: u32,
}

#[cfg(all(feature = "tui", target_os = "linux"))]
impl PtyBrowse {
    /// Start browse on `case` and return once it sleeps in its event loop.
    /// `script` gives the pty no size; without one browse draws empty frames.
    fn start(case: &Case, editor: Option<&std::path::Path>) -> Self {
        use std::io::Read;
        use std::process::{Command, Stdio};

        let pid_file = case.dir.join("browse.pid");
        // `--glyphs unicode`: the assertions name Unicode glyphs, and CI's locale is not UTF-8.
        let inner = format!(
            "stty cols 100 rows 30; echo $$ > '{}'; exec '{}' browse --glyphs unicode --profile '{}' --adapter '{}' --target '{}'",
            pid_file.display(),
            common::binary().display(),
            case.profile.display(),
            case.adapter.display(),
            case.dir.display(),
        );
        let mut command = Command::new("script");
        command
            .args(["-q", "-e", "-c", &inner, "/dev/null"])
            .env("TERM", "xterm")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(editor) = editor {
            command.env("EDITOR", editor);
        }
        let mut script = command.spawn().expect("util-linux `script` runs");

        // The first bytes through the pty mean browse has taken the terminal.
        let mut output = script.stdout.take().unwrap();
        output.read_exact(&mut [0u8; 1]).expect("browse draws");
        let pid = std::fs::read_to_string(&pid_file)
            .expect("the pid file is written before browse starts")
            .trim()
            .parse()
            .unwrap();
        let browse = Self {
            script,
            output,
            pid,
        };
        browse.wait_idle();
        browse
    }

    /// The process state letter from `/proc`, `None` once it is reaped.
    fn state(&self) -> Option<char> {
        std::fs::read_to_string(format!("/proc/{}/stat", self.pid))
            .ok()
            .and_then(|stat| stat.rsplit(") ").next().and_then(|s| s.chars().next()))
    }

    /// True while browse runs. A live process may be in disk sleep (`D`) as well as
    /// `S` or `R`, as under a coverage build writing its profile, so only gone
    /// or a zombie counts as ended.
    fn alive(&self) -> bool {
        !matches!(self.state(), None | Some('Z'))
    }

    /// Wait until browse sleeps in its event loop: a signal or hangup mid-draw
    /// ends it by a different path.
    fn wait_idle(&self) {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.state() != Some('S') && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    fn signal(&self, name: &str) {
        let status = std::process::Command::new("kill")
            .args([&format!("-{name}"), &self.pid.to_string()])
            .status()
            .unwrap();
        assert!(status.success(), "kill -{name} failed");
    }

    fn keys(&mut self, keys: &str) {
        use std::io::Write;
        let stdin = self.script.stdin.as_mut().unwrap();
        stdin.write_all(keys.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// Wait for browse to end; its exit code and what it drew. A browse still
    /// running after the bound is killed, and the test fails.
    ///
    /// The bound is far above a healthy exit because `finish` returns as soon
    /// as browse ends, so slack costs a passing run nothing. What it guards
    /// against, a browse left spinning, exceeds any bound. A tight one failed
    /// in the coverage job (pipeline 7937), where every instrumented process
    /// writes its profile data at exit while the rest of the suite runs.
    fn finish(mut self) -> (Option<i32>, String) {
        use std::io::Read;
        use std::time::{Duration, Instant};
        const BOUND: Duration = Duration::from_secs(30);
        let started = Instant::now();
        while !matches!(self.state(), None | Some('Z')) && started.elapsed() < BOUND {
            std::thread::sleep(Duration::from_millis(50));
        }
        if let state @ Some(letter) = self.state()
            && letter != 'Z'
        {
            let _ = self.script.kill();
            self.signal("KILL");
            let _ = self.script.wait();
            panic!("browse was still running {BOUND:?} later, in state {state:?}");
        }
        let mut drawn = Vec::new();
        let _ = self.output.read_to_end(&mut drawn);
        let code = self.script.wait().unwrap().code();
        (code, String::from_utf8_lossy(&drawn).into_owned())
    }
}

// A terminal that hangs up ends browse rather than leaving it to spin on a dead
// pty. Killing `script` closes the master side, which hangs the terminal up.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn browse_exits_when_its_terminal_hangs_up() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let mut browse = PtyBrowse::start(&case, None);
    browse.script.kill().unwrap();
    browse.script.wait().unwrap();
    // The pty is gone, so neither code nor drawing is left to check.
    let _ = browse.finish();
}

// Each terminating signal ends browse through the event loop: the alternate
// screen is left and the exit code is the last load's, here a clean one.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn browse_restores_the_terminal_on_each_terminating_signal() {
    for signal in ["INT", "QUIT", "TERM", "HUP"] {
        let case = Case::emitting(CLEAN_DOCUMENT);
        let browse = PtyBrowse::start(&case, None);
        browse.signal(signal);
        let (code, drawn) = browse.finish();
        assert_eq!(code, Some(0), "SIG{signal}");
        assert!(
            drawn.contains("\x1b[?1049l"),
            "SIG{signal} left the alternate screen on"
        );
    }
}

// Ctrl-C inside `$EDITOR` interrupts the editor, not browse: as with
// `system()`, an interrupt while the editor runs is not browse's to act on.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn an_interrupt_during_the_editor_does_not_end_browse() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    std::fs::write(case.dir.join("r.md"), "REQ-1\n").unwrap();
    let editor = case.dir.join("editor");
    common::write_stub(&editor, "#!/bin/sh\nkill -INT $PPID\nkill -QUIT $PPID\n");

    let mut browse = PtyBrowse::start(&case, Some(&editor));
    browse.keys("e");
    // Re-entering after the editor asks the terminal where its cursor is;
    // `script` has no terminal behind it, so the test answers.
    std::thread::sleep(std::time::Duration::from_millis(500));
    browse.keys("\x1b[1;1R");
    browse.wait_idle();
    assert!(
        browse.alive(),
        "browse ended with the editor: {:?}",
        browse.state()
    );
    browse.keys("q");
    let (code, drawn) = browse.finish();
    assert_eq!(code, Some(0), "{}", drawn.escape_debug());
}

/// A browse case whose `req` kind declares `comment` running `./tool` with
/// `mode`, and whose adapter reads `doc.json`, so the tool can change the
/// register a reload then reads.
#[cfg(all(feature = "tui", target_os = "linux"))]
fn action_case(mode: &str, tool: &str) -> Case {
    let profile = format!(
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    actions:
      comment: {{ run: ["./tool", "{{id}}"], mode: {mode} }}
edge_kinds:
  derives:
    allowed: [[req, req]]
"#
    );
    let case = Case::with_profile(&profile, r#"cat "$(dirname "$0")/doc.json""#);
    std::fs::write(case.dir.join("doc.json"), CLEAN_DOCUMENT).unwrap();
    std::fs::write(case.dir.join("r.md"), "REQ-1\n").unwrap();
    let path = case.dir.join("tool");
    common::write_stub(&path, &format!("#!/bin/sh\n{tool}\n"));
    case
}

/// Wait for a file a stub writes, and return its trimmed text.
#[cfg(all(feature = "tui", target_os = "linux"))]
fn wait_for(path: &std::path::Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.trim().is_empty()
        {
            return text.trim().to_owned();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// What a reader sees of `drawn`: escape sequences removed and runs of
/// spaces collapsed, since ratatui moves the cursor between changed cells.
#[cfg(all(feature = "tui", target_os = "linux"))]
fn visible(drawn: &str) -> String {
    let mut text = String::new();
    let mut chars = drawn.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next_if_eq(&'[').is_some() {
                // CSI: parameters, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
            text.push(' ');
        } else {
            text.push(c);
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True while `pid` runs and is not a zombie.
#[cfg(all(feature = "tui", target_os = "linux"))]
fn running(pid: &str) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit(") ")
            .next()
            .is_some_and(|s| !s.starts_with('Z'))
    })
}

// Requirement: browse-owner-actions/Interactive mode

// The tool adds a node to the register and exits 3; the reload after it reads
// the new node, and the status line carries both the exit code and the reload.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn an_interactive_action_reloads_after_it_exits() {
    let two_nodes = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}},
            {"id": "REQ-2", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 2}}]}"#;
    let case = action_case("interactive", "cp doc2.json doc.json\nexit 3");
    std::fs::write(case.dir.join("doc2.json"), two_nodes).unwrap();

    let mut browse = PtyBrowse::start(&case, None);
    browse.keys("c");
    browse.wait_idle();
    browse.keys("y");
    std::thread::sleep(std::time::Duration::from_millis(500));
    browse.keys("\x1b[1;1R");
    browse.wait_idle();
    browse.keys("Q");
    let (code, drawn) = browse.finish();
    assert_eq!(code, Some(0), "{}", drawn.escape_debug());
    let seen = visible(&drawn);
    assert!(
        seen.contains("comment exited 3; reloaded: 2 nodes (+1 -0)"),
        "{seen}"
    );
}

// Requirement: browse/Terminal lifecycle

#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn an_interrupt_during_an_interactive_action_does_not_end_browse() {
    let case = action_case("interactive", "kill -INT $PPID\nkill -QUIT $PPID");
    let mut browse = PtyBrowse::start(&case, None);
    browse.keys("c");
    browse.wait_idle();
    browse.keys("y");
    std::thread::sleep(std::time::Duration::from_millis(500));
    browse.keys("\x1b[1;1R");
    browse.wait_idle();
    assert!(
        browse.alive(),
        "browse ended with the action: {:?}",
        browse.state()
    );
    browse.keys("Q");
    let (code, drawn) = browse.finish();
    assert_eq!(code, Some(0), "{}", drawn.escape_debug());
}

// Requirement: browse-owner-actions/Capture mode

// `Esc` stops a capture that is still running: the program is gone and the
// pane says it was stopped.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn esc_stops_a_running_capture() {
    let case = action_case("capture", "echo $$ > tool.pid\nexec sleep 30");
    let mut browse = PtyBrowse::start(&case, None);
    browse.keys("c");
    browse.wait_idle();
    browse.keys("y");
    let pid = wait_for(&case.dir.join("tool.pid"));
    assert!(running(&pid), "the capture is running");
    browse.keys("\x1b");
    browse.wait_idle();
    assert!(!running(&pid), "Esc stopped the program");
    browse.keys("qQ");
    let (code, drawn) = browse.finish();
    assert_eq!(code, Some(0), "{}", drawn.escape_debug());
    let seen = visible(&drawn);
    assert!(seen.contains("./tool REQ-1 — stopped"), "{seen}");
}

// `Ctrl-C` keeps its meaning during a capture: browse quits, and the program
// does not outlive it.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn ctrl_c_during_a_capture_quits_and_stops_the_program() {
    let case = action_case("capture", "echo $$ > tool.pid\nexec sleep 30");
    let mut browse = PtyBrowse::start(&case, None);
    browse.keys("c");
    browse.wait_idle();
    browse.keys("y");
    let pid = wait_for(&case.dir.join("tool.pid"));
    browse.keys("\x03");
    let (code, drawn) = browse.finish();
    assert_eq!(code, Some(0), "{}", drawn.escape_debug());
    assert!(!running(&pid), "the program outlived browse");
}

// A finished capture shows its output in the pane.
#[cfg(all(feature = "tui", target_os = "linux"))]
#[test]
fn a_capture_shows_the_program_output() {
    let case = action_case("capture", "echo $$ > tool.pid\necho \"viewer says $1\"");
    let mut browse = PtyBrowse::start(&case, None);
    browse.keys("c");
    browse.wait_idle();
    browse.keys("y");
    // Browse sleeps while the program runs too, so idle alone does not mean
    // the capture has ended; quitting then races its last frame.
    let pid = wait_for(&case.dir.join("tool.pid"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while running(&pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    browse.wait_idle();
    browse.keys("qQ");
    let (code, drawn) = browse.finish();
    assert_eq!(code, Some(0), "{}", drawn.escape_debug());
    let seen = visible(&drawn);
    assert!(seen.contains("./tool REQ-1 — exited 0"), "{seen}");
    assert!(seen.contains("viewer says REQ-1"), "{seen}");
}

// Requirement: trace-report/Trace report structure

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
    assert!(plain.contains("edges:0 findings:2"), "{plain}");

    let rich = stdout(&case.run(&["trace", "--format", "rich"]));
    assert!(rich.contains("Key Attr"), "{rich}");
    assert!(rich.contains("1 entries, 2 warning(s)"), "{rich}");
}

// Requirement: trace-report/Exit codes for trace

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

// Requirement: trace-report/Trace report structure

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

// Requirement: output/Three output formats

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

#[test]
fn rich_output_forced_color_through_a_pipe_unless_no_color_is_set() {
    // Requirement: output/Three output formats — FORCE_COLOR, and NO_COLOR over it.
    let document = r#"{"interface_version": "1.0",
      "nodes": [],
      "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::with_profile(
        &summary_profile(""),
        &format!("cat <<'DOC'\n{document}\nDOC"),
    );
    let run = |vars: &[(&str, &str)]| {
        let mut command = case.command(&["summary", "--format", "rich"]);
        command.envs(vars.iter().copied());
        command.output().expect("the lattice binary runs")
    };

    let forced = run(&[("FORCE_COLOR", "1")]);
    assert!(stderr(&forced).contains('\u{1b}'), "{}", stderr(&forced));

    let both = run(&[("FORCE_COLOR", "1"), ("NO_COLOR", "1")]);
    assert!(stderr(&both).contains("PARSE_ERROR"), "{}", stderr(&both));
    assert!(
        !stderr(&both).contains('\u{1b}'),
        "NO_COLOR lost to FORCE_COLOR"
    );
}

// Requirement: validation/Severity resolution is shared by every command

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

// Requirement: validation/Findings the pathway pass cannot resolve

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

// Requirement: coverage-query/Summary subcommand

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
    assert_eq!(parsed["groups"][0]["blocked"], 0);
    assert_eq!(parsed["groups"][0]["todo"], 0);
    assert_eq!(parsed["groups"][0]["done"], 1);
    assert_eq!(parsed["totals"]["total"], 1);
}

// Requirement: adapter-contract/Duplicate node IDs are resolved at ingest

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

// Requirement: output/Three output formats

#[test]
fn findings_json_carries_the_node_id() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&case.run(&["validate", "--format", "json"]))).unwrap();
    let unreferenced = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == "UNREFERENCED")
        .unwrap();
    assert_eq!(unreferenced["node_id"], "REQ-1");
}

// Requirement: adapter-contract/Core invokes the adapter with profile and target paths

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

// Requirement: adapter-contract/Adapter failure is a broken setup, not a finding

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
    // with findings. Reading the document would be trusting an aborted run.
    let case = Case::running(&format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC\nexit 1"));
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2);
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
}

// Requirement: validation/Hint severity tier

#[test]
fn hint_only_findings_exit_zero_with_and_without_strict() {
    let profile = format!(
        "{MINIMAL_PROFILE}validations:\n  - UNREFERENCED:\n      severity: hint\n  - UNTRACED:\n      severity: hint\n"
    );
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"));

    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("HINT UNREFERENCED"));
    assert_eq!(
        code(&case.run(&["validate", "--strict", "--format", "plain"])),
        0,
        "a hint is never a reason for a strict run to fail"
    );
    // trace computes its gate separately, over entries and unattachable findings.
    let traced = case.run(&["trace", "--strict", "--format", "plain"]);
    assert_eq!(code(&traced), 0, "{}", stderr(&traced));
}

// Requirement: cli/Profile version pin

#[test]
fn a_matching_profile_version_pin_runs_as_without_it() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let pinned = case.run(&[
        "validate",
        "--profile-version",
        "1.0.0",
        "--format",
        "plain",
    ]);
    let unpinned = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&pinned), 0, "{}", stderr(&pinned));
    assert_eq!(stdout(&pinned), stdout(&unpinned));
}

#[test]
fn a_mismatched_profile_version_pin_exits_two_naming_both_versions() {
    // ERROR_DOCUMENT would exit 1 if it were read: exit 2 proves the refusal
    // comes before the adapter, and is never collapsed into a finding.
    let case = Case::emitting(ERROR_DOCUMENT);
    for command in [
        &["validate"][..],
        &["summary"],
        &["trace"],
        &["coverage"],
        &["query", "counts"],
    ] {
        let mut args = command.to_vec();
        args.extend(["--profile-version", "1.1.0", "--format", "plain"]);
        let output = case.run(&args);
        assert_eq!(code(&output), 2, "{command:?}: {}", stderr(&output));
        assert!(
            stdout(&output).is_empty(),
            "{command:?}: {}",
            stdout(&output)
        );
        let err = stderr(&output);
        assert!(
            err.starts_with("Error: Profile error:"),
            "{command:?}: {err}"
        );
        assert!(err.contains("'1.0.0'") && err.contains("'1.1.0'"), "{err}");
    }
}

#[test]
fn a_malformed_profile_version_pin_exits_two() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    let output = case.run(&["validate", "--profile-version", "1.0", "--format", "plain"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stdout(&output).is_empty());
    assert!(
        stderr(&output).contains("is not a valid version"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn show_honours_the_profile_version_pin() {
    let case = Case::emitting("{}");
    let show = |pin: &str| {
        common::lattice()
            .arg("show")
            .arg("--profile")
            .arg(&case.profile)
            .args(["--profile-version", pin])
            .output()
            .expect("the lattice binary runs")
    };
    assert_eq!(code(&show("1.0.0")), 0);
    let refused = show("1.0.1");
    assert_eq!(code(&refused), 2, "{}", stderr(&refused));
    assert!(refused.stdout.is_empty(), "no handoff document is printed");
    assert!(stderr(&refused).contains("'1.0.1'"), "{}", stderr(&refused));
}

// Requirement: cli/Show command

#[test]
fn show_prints_the_handoff_document_an_adapter_reads() {
    let case = Case::emitting("{}");
    let output = case_show(&case);

    assert_eq!(output.status.code(), Some(0));
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is the resolved JSON document");
    assert_eq!(parsed["resolved_schema"], "1");
    assert!(parsed["node_kinds"]["req"].is_object(), "{parsed}");
}

/// The point of the command: a program that consumes a register can be run by
/// hand on what it prints, without reimplementing profile resolution.
#[test]
fn an_adapter_runs_on_what_show_printed() {
    let case = Case::emitting("{}");
    let resolved = case.dir.join("resolved.json");
    std::fs::write(&resolved, case_show(&case).stdout).unwrap();

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
fn show_on_an_unloadable_profile_is_exit_two() {
    let case = Case::with_profile("name: t\nnode_kinds: 3\n", "true");
    let output = case_show(&case);

    assert_eq!(output.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}

/// The command was `resolve` before 0.8.0. It has no alias: the old name is
/// refused like any unknown command, so a stale script stops rather than runs.
#[test]
fn the_old_resolve_name_is_refused() {
    let case = Case::emitting("{}");
    let output = common::lattice()
        .arg("resolve")
        .arg("--profile")
        .arg(&case.profile)
        .output()
        .expect("the lattice binary runs");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "no handoff document is printed");
}

/// `show` takes neither `--adapter` nor `--target`, so it cannot use
/// `Case::run`, which supplies both.
fn case_show(case: &Case) -> std::process::Output {
    common::lattice()
        .arg("show")
        .arg("--profile")
        .arg(&case.profile)
        .output()
        .expect("the lattice binary runs")
}

// Requirement: cli/Exit codes

/// The minimal profile plus the given `validations` entries.
fn suppressing(validations: &str) -> String {
    format!("{MINIMAL_PROFILE}validations:\n{validations}")
}

/// Two dangling edges from one node: two error-severity VACANCYs, both with
/// `node_id` REQ-1, distinguishable only by their target.
const TWO_ERRORS_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}},
            {"id": "REQ-2", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 2}}],
  "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
             "provenance": {"file": "r.md", "line": 1}},
            {"src": "REQ-2", "tgt": "REQ-8", "kind": "derives",
             "provenance": {"file": "r.md", "line": 2}}]}"#;

#[test]
fn suppressed_errors_do_not_gate() {
    let case = Case::with_profile(
        &suppressing("  - SUPPRESS: {code: VACANCY}\n"),
        &format!("cat <<'DOC'\n{ERROR_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(!stdout(&output).contains("VACANCY"), "{}", stdout(&output));

    let json = case.run(&["validate", "--format", "json"]);
    assert_eq!(code(&json), 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    let vacancy = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == "VACANCY")
        .expect("json keeps the audit trail");
    assert_eq!(vacancy["suppressed"], true);
    assert_eq!(vacancy["severity"], "error");

    // trace gates on the same terms.
    assert_eq!(code(&case.run(&["trace", "--format", "plain"])), 0);
}

#[test]
fn one_unsuppressed_error_still_gates() {
    let case = Case::with_profile(
        &suppressing("  - SUPPRESS: {code: VACANCY, node_ids: [REQ-1]}\n"),
        &format!("cat <<'DOC'\n{TWO_ERRORS_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 1);
    let shown = stdout(&output);
    assert!(shown.contains("REQ-8"), "{shown}");
    assert!(!shown.contains("REQ-9"), "{shown}");
}

#[test]
fn strict_promoted_then_suppressed_does_not_gate() {
    let case = Case::with_profile(
        &suppressing("  - SUPPRESS: {code: UNREFERENCED}\n  - SUPPRESS: {code: UNTRACED}\n"),
        &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["validate", "--strict", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    for finding in parsed["findings"].as_array().unwrap() {
        if finding["code"] == "EMPTY_KIND" {
            // Info, which `--strict` leaves alone.
            continue;
        }
        assert_eq!(finding["severity"], "error", "strict still promoted it");
        assert_eq!(finding["suppressed"], true);
    }
}

#[test]
fn a_stale_suppress_entry_is_reported_at_info_and_exits_zero() {
    let case = Case::with_profile(
        &suppressing("  - SUPPRESS: {code: VACANCY}\n"),
        &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("INFO SUPPRESS_UNUSED <profile>:0"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_profile_suppressing_config_error_exits_two() {
    let case = Case::with_profile(
        &suppressing("  - SUPPRESS: {code: CONFIG_ERROR}\n"),
        &format!("cat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("CONFIG_ERROR"),
        "{}",
        stderr(&output)
    );
}

// Requirement: adapter-contract/The adapter program's run is bounded

#[test]
fn adapter_exceeding_its_bound_exits_two() {
    let profile = format!("{MINIMAL_PROFILE}adapter:\n  timeout_seconds: 1\n");
    let case = Case::with_profile(
        &profile,
        &format!("exec sleep 30\ncat <<'DOC'\n{CLEAN_DOCUMENT}\nDOC"),
    );
    let started = std::time::Instant::now();
    let output = case.run(&["validate", "--format", "plain"]);

    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the bound, not the sleep, decides when the run ends"
    );
    assert_eq!(code(&output), 2);
    let message = stderr(&output);
    assert!(message.contains("exceeded 1 s"), "{message}");
    assert!(
        message.contains(&format!("adapter '{}'", case.adapter.display())),
        "{message}"
    );
    assert!(stdout(&output).is_empty(), "nothing was ingested");
}

// Requirement: adapter-contract/Document schema

#[test]
fn provenance_line_beyond_32_bits_exits_two_and_names_the_line() {
    let case = Case::emitting(
        r#"{"interface_version": "1.4",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 4294967296}}]}"#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("'line'") && stderr(&output).contains("got 4294967296"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn provenance_line_at_the_32_bit_maximum_is_accepted() {
    let case = Case::emitting(
        r#"{"interface_version": "1.4",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 4294967295}}]}"#,
    );
    let output = case.run(&["validate", "--format", "plain"]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("r.md:4294967295"),
        "{}",
        stdout(&output)
    );
}

// Requirement: output/Validate reports its scope

/// Three requirements from one file, a declared `test` kind and a declared
/// `verifies` kind the adapter never produced.
const SCOPE_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  requirement:
    id_pattern: "^REQ-\\d+$"
  test:
    id_pattern: "^T-\\d+$"
edge_kinds:
  verifies:
    allowed: [[test, requirement]]
"#;

const SCOPE_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "requirement", "attrs": {}, "provenance": {"file": "r.md", "line": 1}},
            {"id": "REQ-2", "kind": "requirement", "attrs": {}, "provenance": {"file": "r.md", "line": 2}},
            {"id": "REQ-3", "kind": "requirement", "attrs": {}, "provenance": {"file": "r.md", "line": 3}}]}"#;

#[test]
fn validate_scope_in_json_counts_every_declared_kind_and_the_files_read() {
    let case = Case::with_profile(
        SCOPE_PROFILE,
        &format!("cat <<'DOC'\n{SCOPE_DOCUMENT}\nDOC"),
    );
    let output = case.run(&["validate", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(
        parsed["scope"],
        serde_json::json!({"edges": {"verifies": 0}, "files": 1,
                           "nodes": {"requirement": 3, "test": 0}})
    );
    assert!(parsed["findings"].is_array(), "{parsed}");
}

/// Six nodes in two files, every declared kind used, no finding: the plain
/// run writes nothing to stdout and one scope line to stderr.
#[test]
fn validate_scope_is_one_stderr_line_on_a_clean_plain_run() {
    let document = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "requirement", "attrs": {}, "provenance": {"file": "r.md", "line": 1}},
            {"id": "REQ-2", "kind": "requirement", "attrs": {}, "provenance": {"file": "r.md", "line": 2}},
            {"id": "REQ-3", "kind": "requirement", "attrs": {}, "provenance": {"file": "r.md", "line": 3}},
            {"id": "T-1", "kind": "test", "attrs": {}, "provenance": {"file": "t.md", "line": 1}},
            {"id": "T-2", "kind": "test", "attrs": {}, "provenance": {"file": "t.md", "line": 2}},
            {"id": "T-3", "kind": "test", "attrs": {}, "provenance": {"file": "t.md", "line": 3}}],
  "edges": [{"src": "T-1", "tgt": "REQ-1", "kind": "verifies", "provenance": {"file": "t.md", "line": 1}},
            {"src": "T-2", "tgt": "REQ-2", "kind": "verifies", "provenance": {"file": "t.md", "line": 2}},
            {"src": "T-3", "tgt": "REQ-3", "kind": "verifies", "provenance": {"file": "t.md", "line": 3}}]}"#;
    let case = Case::with_profile(SCOPE_PROFILE, &format!("cat <<'DOC'\n{document}\nDOC"));
    for format in ["plain", "rich"] {
        let output = case.run(&["validate", "--format", format]);
        assert_eq!(code(&output), 0, "{format}: {}", stderr(&output));
        if format == "plain" {
            assert_eq!(stdout(&output), "", "a clean plain run keeps stdout empty");
        }
        assert_eq!(
            stderr(&output),
            "scope: 2 files; nodes requirement 3, test 3; edges verifies 3\n",
            "{format}"
        );
    }
}

// Requirement: cli/A missing profile names the profiles beside it

#[test]
fn missing_profile_names_the_profiles_beside_it() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    // The case's own profile.yaml would be a candidate too; the scenario has one.
    std::fs::remove_file(&case.profile).unwrap();
    std::fs::write(case.dir.join("lattice.yaml"), MINIMAL_PROFILE).unwrap();
    std::fs::write(
        case.dir.join("notes.yaml"),
        "- finding: {code: VACANCY, node: REQ-5}\n  note: x\n",
    )
    .unwrap();
    // A bare name resolves against the working directory, so the scan does too.
    let output = lattice()
        .args(["validate", "--profile", "lattice.yml", "--target", "."])
        .arg("--adapter")
        .arg(&case.adapter)
        .current_dir(&case.dir)
        .output()
        .expect("the lattice binary runs");
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("lattice.yaml"), "{message}");
    assert!(!message.contains("notes.yaml"), "{message}");
    assert!(!message.contains("profile.yaml"), "{message}");
}

#[test]
fn missing_profile_with_no_candidate_says_so() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    std::fs::remove_file(&case.profile).unwrap();
    std::fs::write(case.dir.join("notes.yaml"), "- finding: {code: VACANCY}\n").unwrap();
    let output = case.run(&["validate", "--format", "plain"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("no profile found in"), "{message}");
    assert!(
        message.contains(&case.dir.display().to_string()),
        "{message}"
    );
    assert!(!message.contains("notes.yaml"), "{message}");
}

// Requirement: describe-command/Describe command

const DESCRIBE_DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "requirement", "attrs": {}, "provenance": {"file": "a.md", "line": 1}},
            {"id": "REQ-2", "kind": "requirement", "attrs": {}, "provenance": {"file": "b.md", "line": 1}}]}"#;

#[test]
fn describe_orients_on_a_register_in_json() {
    let profile = format!(
        "{SCOPE_PROFILE}adapter:\n  timeout_seconds: 7\nvalidations:\n  - SUPPRESS: {{code: UNREFERENCED}}\n  - UNTRACED:\n      severity: info\n"
    );
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{DESCRIBE_DOCUMENT}\nDOC"));
    let output = case.run(&["describe", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(parsed["profile"]["name"], "t");
    // The block as the adapter receives it: the core interprets none of its keys.
    assert_eq!(
        parsed["adapter"]["config"],
        serde_json::json!({"timeout_seconds": 7})
    );
    assert!(
        parsed["adapter"]["name"]
            .as_str()
            .unwrap()
            .ends_with("adapter"),
        "{parsed}"
    );
    assert_eq!(
        parsed["node_kinds"],
        serde_json::json!([
            {"kind": "requirement", "id_pattern": "^REQ-\\d+$", "count": 2, "files": ["a.md", "b.md"]},
            {"kind": "test", "id_pattern": "^T-\\d+$", "count": 0, "files": []}
        ])
    );
    assert_eq!(
        parsed["edge_kinds"],
        serde_json::json!([{"kind": "verifies", "allowed": [["test", "requirement"]], "count": 0}])
    );
    let codes: Vec<&str> = parsed["validations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["SUPPRESS", "UNTRACED"]);
    assert_eq!(parsed["findings"], serde_json::json!([]));
}

#[test]
fn describe_reports_an_adapter_error_finding_and_still_exits_zero() {
    let document = r#"{"interface_version": "1.0", "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "bad row",
                  "provenance": {"file": "r.md", "line": 3}, "node_id": null}]}"#;
    let case = Case::with_profile(SCOPE_PROFILE, &format!("cat <<'DOC'\n{document}\nDOC"));
    for format in ["plain", "json", "rich"] {
        let output = case.run(&["describe", "--format", format]);
        assert_eq!(code(&output), 0, "{format}: {}", stderr(&output));
        assert!(
            stdout(&output).contains("PARSE_ERROR"),
            "{format}: {}",
            stdout(&output)
        );
    }
}

#[test]
fn describe_applies_the_profile_message_to_an_adapter_finding_as_validate_does() {
    let document = r#"{"interface_version": "1.0", "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "bad row",
                  "provenance": {"file": "r.md", "line": 3}, "node_id": null}]}"#;
    let profile = format!(
        "{SCOPE_PROFILE}validations:\n  - PARSE_ERROR:\n      message: 'repair r.md: {{message}}'\n"
    );
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{document}\nDOC"));
    for format in ["plain", "json", "rich"] {
        let validated = case.run(&["validate", "--format", format]);
        assert!(
            stdout(&validated).contains("repair r.md: bad row"),
            "validate {format}: {}",
            stdout(&validated)
        );
        let described = case.run(&["describe", "--format", format]);
        assert_eq!(code(&described), 0, "{format}: {}", stderr(&described));
        assert!(
            stdout(&described).contains("repair r.md: bad row"),
            "describe {format}: {}",
            stdout(&described)
        );
    }
}

#[test]
fn describe_carries_the_adapter_block_as_a_json_object() {
    let profile = format!(
        "{SCOPE_PROFILE}adapter:\n  timeout_seconds: 7\n  paths:\n    files: ['*.md', 'docs/*.md']\n"
    );
    let case = Case::with_profile(&profile, &format!("cat <<'DOC'\n{DESCRIBE_DOCUMENT}\nDOC"));
    let output = case.run(&["describe", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    // The object the adapter receives in the resolved profile, so a script reads
    // the globs without a YAML parser.
    assert_eq!(
        parsed["adapter"]["config"],
        serde_json::json!({"timeout_seconds": 7, "paths": {"files": ["*.md", "docs/*.md"]}})
    );
    // Plain and rich keep the block as YAML for a reader.
    for format in ["plain", "rich"] {
        let output = case.run(&["describe", "--format", format]);
        assert!(
            stdout(&output).contains("timeout_seconds: 7"),
            "{format}: {}",
            stdout(&output)
        );
    }
}

#[test]
fn describe_exits_two_when_the_profile_does_not_load() {
    let case = Case::emitting(CLEAN_DOCUMENT);
    std::fs::write(&case.profile, "name: t\nnode_kinds: {}\n").unwrap();
    let output = case.run(&["describe", "--format", "plain"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Profile error"),
        "{}",
        stderr(&output)
    );
}
