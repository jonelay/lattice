//! Every command's `plain` and `rich` output over a register whose every string
//! carries terminal controls: the bytes written must hold none of them raw.
#![cfg(unix)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::{Case, code, lattice, stderr};

/// The hostile text, as YAML double-quoted escapes and as JSON escapes: an
/// OSC 8 link opener, a newline, a carriage return and a right-to-left override.
const YAML_HOSTILE: &str = r"\e]8;;x\a\n\r\u202E";
const JSON_HOSTILE: &str = r"\u001b]8;;x\u0007\n\r\u202e";

/// What the hostile text's tail looks like escaped. A line that starts with it
/// had a raw newline written in the middle of a field.
const SPLIT_TAIL: &str = r"\r\u{202e}";

/// The escape sequences `rich` may write when colour is on: the severity
/// colours and the reset.
const SGR: [&str; 5] = [
    "\u{1b}[31m",
    "\u{1b}[33m",
    "\u{1b}[36m",
    "\u{1b}[35m",
    "\u{1b}[0m",
];

const SEVERITIES: [&str; 4] = ["ERROR", "WARNING", "INFO", "HINT"];

/// The profile: every kind, attr, edge kind and pathway name is hostile.
fn profile() -> String {
    let h = YAML_HOSTILE;
    format!(
        r#"name: h
profile_version: "1.0.0"
annotations: notes.yaml
pathways: ["ph{h}"]
node_kinds:
  "k{h}":
    id_pattern: "(?s).*"
    summary_attr: "s{h}"
    text_attrs: ["s{h}", "t{h}"]
    attrs:
      "s{h}": {{type: string}}
      "t{h}": {{type: string}}
      "p{h}": {{type: string}}
edge_kinds:
  "e{h}":
    allowed: [["k{h}", "k{h}"]]
validations:
  - COVERAGE: {{target_kind: "k{h}", edge_kind: "e{h}", severity: warning}}
  - VACANCY: {{pathway: "ph{h}", position_attr: "p{h}"}}
"#
    )
}

/// A note on the dangling edge's finding, with hostile text and author.
fn notes() -> String {
    let h = YAML_HOSTILE;
    format!("- finding: {{code: VACANCY, node: \"A{h}\"}}\n  note: \"n{h}\"\n  by: \"b{h}\"\n")
}

/// The interface document: hostile node IDs, kinds, attr names and values,
/// edge kinds and targets (one dangling), pathway, provenance and a finding.
/// `extra` adds a node, so two revisions differ.
fn document(extra: bool) -> String {
    let j = JSON_HOSTILE;
    let d = if extra {
        format!(
            r#",{{"id": "D{j}", "kind": "k{j}", "attrs": {{"s{j}": "new{j}"}}, "provenance": {{"file": "g{j}.md", "line": 4}}}}"#
        )
    } else {
        String::new()
    };
    format!(
        r#"{{"interface_version": "1.0",
 "nodes": [
  {{"id": "A{j}", "kind": "k{j}", "attrs": {{"s{j}": "sum{j}", "t{j}": "txt{j}", "p{j}": "M0{j}"}}, "provenance": {{"file": "f{j}.md", "line": 1}}}},
  {{"id": "B{j}", "kind": "k{j}", "attrs": {{"s{j}": "sum{j}", "t{j}": "txt{j}", "p{j}": "M1{j}"}}, "provenance": {{"file": "f{j}.md", "line": 2}}}},
  {{"id": "C{j}", "kind": "k{j}", "attrs": {{"s{j}": "sum{j}"}}, "provenance": {{"file": "g{j}.md", "line": 3}}}}{d}],
 "edges": [
  {{"src": "A{j}", "tgt": "B{j}", "kind": "e{j}", "provenance": {{"file": "f{j}.md", "line": 1}}}},
  {{"src": "A{j}", "tgt": "M{j}", "kind": "e{j}", "provenance": {{"file": "f{j}.md", "line": 1}}}}],
 "pathways": [{{"name": "ph{j}", "order": ["M0{j}", "M1{j}"], "current": "M0{j}"}}],
 "findings": [{{"severity": "warning", "code": "PARSE_ERROR", "message": "m{j}", "provenance": {{"file": "f{j}.md", "line": 9}}, "node_id": "A{j}"}}]}}"#
    )
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The case: the target is a git repository whose first commit lacks node D.
fn hostile_case() -> (Case, String, String) {
    let case = Case::with_profile(&profile(), "cat \"$4/doc.json\"");
    std::fs::write(case.dir.join("notes.yaml"), notes()).unwrap();
    git(&case.dir, &["init", "-q"]);
    std::fs::write(case.dir.join("doc.json"), document(false)).unwrap();
    git(&case.dir, &["add", "doc.json"]);
    git(&case.dir, &["commit", "-q", "-m", "a"]);
    let rev_a = git(&case.dir, &["rev-parse", "HEAD"]);
    std::fs::write(case.dir.join("doc.json"), document(true)).unwrap();
    git(&case.dir, &["commit", "-q", "-a", "-m", "b"]);
    let rev_b = git(&case.dir, &["rev-parse", "HEAD"]);
    let h = YAML_HOSTILE;
    std::fs::write(
        case.dir.join("fuse.yaml"),
        format!(
            "manifest_version: '1.0.0'\nname: hostile\nversion: '1.0.0'\nfuse_profile: fuse-profile.yaml\n\
             sources:\n  - {{name: \"s{h}\", profile: profile.yaml, adapter: adapter, target: .}}\n"
        ),
    )
    .unwrap();
    std::fs::write(case.dir.join("fuse-profile.yaml"), "edge_kinds: {}\n").unwrap();
    (case, rev_a, rev_b)
}

/// What the section 4.3 byte checker rejects: invalid UTF-8, a control but
/// newline and tab, DEL, C1, the bidi controls and the line separators, an
/// escape other than `rich`'s own colours (`sgr`), and a hostile field cut in
/// two by a raw newline.
fn violation(bytes: &[u8], sgr: bool) -> Option<String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => return Some(format!("invalid UTF-8: {error}")),
    };
    let text = if sgr {
        SGR.iter().fold(text.to_string(), |t, s| t.replace(s, ""))
    } else {
        text.to_string()
    };
    if let Some(c) = text.chars().find(|&c| {
        (c < ' ' && c != '\n' && c != '\t')
            || ('\u{7f}'..='\u{9f}').contains(&c)
            || matches!(
                c,
                '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}'
            )
    }) {
        return Some(format!("raw {c:?}"));
    }
    if let Some(line) = text.lines().find(|line| line.starts_with(SPLIT_TAIL)) {
        return Some(format!("a field was split across lines: {line:?}"));
    }
    None
}

/// Lines whose first word is a severity, colour removed.
fn severity_lines(bytes: &[u8]) -> usize {
    let text = SGR
        .iter()
        .fold(String::from_utf8_lossy(bytes).into_owned(), |t, s| {
            t.replace(s, "")
        });
    text.lines()
        .filter(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|word| SEVERITIES.contains(&word))
        })
        .count()
}

fn run(mut command: Command, format: &str, env: Option<(&str, &str)>) -> Output {
    command.args(["--format", format]);
    if let Some((name, value)) = env {
        command.env(name, value);
    }
    command.output().expect("the lattice binary runs")
}

// Requirement: output/Untrusted text in human formats
// The status rollup view: hostile group and status values and a hostile
// grouping attr name, each row still one line.
#[test]
fn the_summary_rollup_writes_no_raw_control() {
    let h = YAML_HOSTILE;
    let rollup = format!(
        "{}  - SUMMARY: {{node_kind: \"k{h}\", status_attr: \"s{h}\", group_by_attr: \"t{h}\"}}\n",
        profile()
    );
    let case = Case::with_profile(&rollup, "cat \"$4/doc.json\"");
    std::fs::write(case.dir.join("notes.yaml"), notes()).unwrap();
    std::fs::write(case.dir.join("doc.json"), document(false)).unwrap();
    let json = run(case.command(&["summary"]), "json", None);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let groups = value["groups"].as_array().unwrap().len();
    assert!(groups > 0, "{value}");
    let mut failures = Vec::new();
    for (format, env, sgr, frame) in [
        ("plain", None, false, 0),
        ("rich", Some(("NO_COLOR", "1")), false, 4),
        ("rich", Some(("FORCE_COLOR", "1")), true, 4),
    ] {
        let output = run(case.command(&["summary"]), format, env);
        assert_eq!(code(&output), 0, "{format}: {}", stderr(&output));
        for (stream, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
            if let Some(problem) = violation(bytes, sgr) {
                failures.push(format!("{format} {env:?} {stream}: {problem}"));
            }
        }
        let lines = String::from_utf8_lossy(&output.stdout).lines().count();
        if lines != groups + frame {
            failures.push(format!(
                "{format} {env:?}: {lines} lines for {groups} groups"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Requirement: output/Untrusted text in human formats
#[test]
fn no_command_writes_a_raw_control() {
    let (case, rev_a, rev_b) = hostile_case();
    let j = |s: &str| s.replace(JSON_HOSTILE, "\u{1b}]8;;x\u{7}\n\r\u{202e}");
    let a = j(&format!("A{JSON_HOSTILE}"));
    let b = j(&format!("B{JSON_HOSTILE}"));
    let file = j(&format!("f{JSON_HOSTILE}.md"));
    let commands: Vec<Vec<&str>> = vec![
        vec!["validate"],
        vec!["trace"],
        vec!["summary"],
        vec!["describe"],
        vec!["coverage"],
        vec!["coverage", "--rule", "1"],
        vec!["query", "at", &file],
        vec!["query", "reaches", &a],
        vec!["query", "reached-by", &b],
        vec!["query", "path", &a, &b],
        vec!["query", "orphans"],
        vec!["query", "counts"],
        vec!["query", "diff", &rev_a, &rev_b],
    ];
    let mut failures = Vec::new();
    let mut check = |name: &str, output: &Output, sgr: bool| {
        assert!(
            matches!(code(output), 0 | 1),
            "{name}: exit {}: {}",
            code(output),
            stderr(output)
        );
        for (stream, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
            if let Some(problem) = violation(bytes, sgr) {
                failures.push(format!("{name} {stream}: {problem}"));
            }
        }
    };
    let variants = [
        ("plain", None, false),
        ("rich", Some(("NO_COLOR", "1")), false),
        ("rich", Some(("FORCE_COLOR", "1")), true),
    ];
    for args in &commands {
        for (format, env, sgr) in variants {
            let output = run(case.command(args), format, env);
            check(&format!("{args:?} {format} {env:?}"), &output, sgr);
        }
    }
    for (format, env, sgr) in variants {
        let mut command = lattice();
        command
            .args(["fuse", "--manifest"])
            .arg(case.dir.join("fuse.yaml"));
        let output = run(command, format, env);
        check(&format!("fuse {format} {env:?}"), &output, sgr);
    }

    // A forged finding line would be one more severity-first line than json has.
    let json = run(case.command(&["validate"]), "json", None);
    let findings: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let count = findings["findings"].as_array().unwrap().len();
    assert!(count > 0);
    for (format, env, _) in variants {
        let output = run(case.command(&["validate"]), format, env);
        assert_eq!(
            severity_lines(&output.stdout),
            count,
            "validate {format} {env:?}"
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
